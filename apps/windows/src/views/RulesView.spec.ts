import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { createPinia, setActivePinia } from "pinia";
import { mountApp } from "@/test/harness";
import RulesView from "@/views/RulesView.vue";
import { useCoreStore } from "@/stores/core";

const sample = [
  { type: "DomainSuffix", payload: "openai.com", proxy: "人工智能" },
  { type: "DomainSuffix", payload: "doubleclick.net", proxy: "REJECT" },
  { type: "IPCIDR", payload: "192.168.0.0/16", proxy: "DIRECT" },
  { type: "Match", payload: "", proxy: "扶梯出行" },
];

function setup(opts: { running?: boolean; rules?: unknown[] | Error } = {}) {
  const pinia = createPinia();
  setActivePinia(pinia);
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "get_rules") {
      return opts.rules instanceof Error ? Promise.reject(opts.rules) : Promise.resolve(opts.rules ?? sample);
    }
    return Promise.resolve(undefined);
  });
  useCoreStore().status = { running: opts.running ?? true, status: "running", version: null };
  return pinia;
}

async function mountView(opts: Parameters<typeof setup>[0] = {}) {
  const pinia = setup(opts);
  const w = mountApp(RulesView, { attachTo: document.body }, pinia);
  await flushPromises();
  return w;
}

beforeEach(() => {
  document.body.innerHTML = "";
});

describe("RulesView", () => {
  it("shows an empty state instead of calling the backend when the core is not running", async () => {
    const w = await mountView({ running: false });
    expect(w.find(".rules-empty").text()).toContain("rules.core_not_running");
    expect(invokeMock).not.toHaveBeenCalledWith("get_rules");
    w.unmount();
  });

  it("renders rules as grid rows (no table) with type / payload / policy columns", async () => {
    const w = await mountView();
    expect(w.find("table").exists()).toBe(false);
    const rows = w.findAll(".rule-list li");
    expect(rows).toHaveLength(4);
    expect(rows[0].text()).toContain("DomainSuffix");
    expect(rows[0].text()).toContain("openai.com");
    expect(w.find(".count").text()).toBe("rules.count");
    // Match 规则没有 payload：显示占位而不是空白
    expect(rows[3].find(".payload").text()).toBe("—");
    w.unmount();
  });

  it("colours policies: DIRECT secondary, REJECT danger, groups accent", async () => {
    const w = await mountView();
    const cls = w.findAll(".policy").map((p) => p.classes().find((c) => c.startsWith("is-")));
    expect(cls).toEqual(["is-proxy", "is-reject", "is-direct", "is-proxy"]);
    w.unmount();
  });

  it("filters by type, content or policy while keeping the original row number", async () => {
    const w = await mountView();
    await w.find(".search-box input").setValue("direct");
    let rows = w.findAll(".rule-list li");
    expect(rows).toHaveLength(1);
    expect(rows[0].find(".idx").text()).toBe("3");
    await w.find(".search-box input").setValue("ipcidr");
    expect(w.findAll(".rule-list li")).toHaveLength(1);
    await w.find(".search-box input").setValue("人工");
    rows = w.findAll(".rule-list li");
    expect(rows).toHaveLength(1);
    await w.find(".search-box input").setValue("zzz");
    expect(w.find(".rules-empty").text()).toContain("rules.no_match");
    w.unmount();
  });

  it("paginates beyond 500 rules instead of rendering them all", async () => {
    const many = Array.from({ length: 1234 }, (_, i) => ({ type: "Domain", payload: `d${i}.com`, proxy: "DIRECT" }));
    const w = await mountView({ rules: many });
    expect(w.findAll(".rule-list li")).toHaveLength(500);
    expect(w.find(".pager").text()).toContain("1 / 3");
    const next = w.findAll(".pager button").find((b) => b.text() === "rules.page_next")!;
    await next.trigger("click");
    await next.trigger("click");
    expect(w.findAll(".rule-list li")).toHaveLength(234);
    expect(w.findAll(".rule-list li")[0].find(".idx").text()).toBe("1001");
    expect(next.attributes("disabled")).toBeDefined();
    w.unmount();
  });

  it("is read-only: no editing controls", async () => {
    const w = await mountView();
    expect(w.findAll("button")).toHaveLength(0);
    w.unmount();
  });

  it("shows an empty state for an empty rule list and a failure state on error", async () => {
    const empty = await mountView({ rules: [] });
    expect(empty.find(".rules-empty").text()).toContain("rules.empty");
    empty.unmount();
    const bad = await mountView({ rules: new Error("boom") });
    expect(bad.find(".rules-empty").text()).toContain("rules.load_failed");
    bad.unmount();
  });
});
