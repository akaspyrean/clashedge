import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";

const handlers: Record<string, (e: { payload: unknown }) => void> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = cb;
    return Promise.resolve(() => {});
  },
}));
const invokeMock = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
const confirmMock = vi.fn();
vi.mock("@/composables/useConfirm", () => ({ useConfirm: () => confirmMock }));

import { createPinia, setActivePinia } from "pinia";
import { mountApp } from "@/test/harness";
import LogsView from "@/views/LogsView.vue";
import { useCoreStore } from "@/stores/core";

const emit = (level: string, message: string) => handlers["log-line"]({ payload: { level, message } });

async function mountView() {
  const pinia = createPinia();
  setActivePinia(pinia);
  useCoreStore().status = { running: true, status: "running", version: null };
  const w = mountApp(LogsView, { attachTo: document.body }, pinia);
  await flushPromises();
  emit("info", "[TCP] a --> openai.com:443");
  emit("warning", "[Provider] update failed");
  emit("error", "dial timeout");
  emit("debug", "dns resolve ok");
  emit("info", "[TCP] b --> github.com:443");
  await flushPromises();
  return w;
}

beforeEach(() => {
  document.body.innerHTML = "";
  confirmMock.mockReset();
});

describe("LogsView", () => {
  it("lays lines out as time | level | message in mono", async () => {
    const w = await mountView();
    const first = w.find(".log-line");
    expect(first.find(".log-time").text()).toMatch(/^\d{2}:\d{2}:\d{2}$/);
    expect(first.find(".log-level").text()).toBe("INFO");
    expect(first.find(".log-msg").text()).toContain("openai.com");
    w.unmount();
  });

  it("level chips carry counts and a shape per level (never colour alone)", async () => {
    const w = await mountView();
    const chips = w.findAll(".chip").map((c) => c.text().replace(/\s+/g, " "));
    expect(chips).toEqual(["logs.level_all 5", "DEBUG 1", "INFO 2", "WARN 1", "ERROR 1"]);
    expect(w.findAll(".chip .mark-info, .chip .mark-warning, .chip .mark-error")).toHaveLength(3);
    w.unmount();
  });

  it("filters by level and by regex; an invalid regex falls back to substring", async () => {
    const w = await mountView();
    await w.findAll(".chip")[3].trigger("click"); // WARN
    expect(w.findAll(".log-line")).toHaveLength(1);
    await w.findAll(".chip")[0].trigger("click"); // all
    await w.find(".search-box input").setValue("(openai|github)\\.com");
    expect(w.findAll(".log-line")).toHaveLength(2);
    await w.find(".search-box input").setValue("[unclosed");
    expect(w.find(".log-placeholder").text()).toBe("logs.no_match");
    await w.find(".search-box input").setValue("dns");
    expect(w.findAll(".log-line")).toHaveLength(1);
    w.unmount();
  });

  it("pause freezes the list; resume merges what arrived meanwhile", async () => {
    const w = await mountView();
    const pause = w.find(".pause-btn");
    await pause.trigger("click");
    emit("info", "arrived while paused");
    await flushPromises();
    expect(w.findAll(".log-line")).toHaveLength(5);
    expect(pause.attributes("aria-pressed")).toBe("true");
    await pause.trigger("click");
    expect(w.findAll(".log-line")).toHaveLength(6);
    w.unmount();
  });

  it("clear… asks first; cancelling keeps the lines", async () => {
    const w = await mountView();
    confirmMock.mockResolvedValueOnce(false);
    await w.find(".clear-btn").trigger("click");
    await flushPromises();
    expect(confirmMock.mock.calls[0][1]).toMatchObject({ danger: true });
    expect(w.findAll(".log-line")).toHaveLength(5);
    confirmMock.mockResolvedValueOnce(true);
    await w.find(".clear-btn").trigger("click");
    await flushPromises();
    expect(w.findAll(".log-line")).toHaveLength(0);
    w.unmount();
  });

  it("shows a jump-to-latest button once the view is scrolled away from the bottom", async () => {
    const w = await mountView();
    expect(w.find(".jump").exists()).toBe(false);
    const el = w.find(".log-scroll").element as HTMLElement;
    Object.defineProperty(el, "scrollHeight", { configurable: true, value: 1000 });
    Object.defineProperty(el, "clientHeight", { configurable: true, value: 300 });
    el.scrollTop = 100;
    await w.find(".log-scroll").trigger("scroll");
    expect(w.find(".jump").exists()).toBe(true);
    await w.find(".jump").trigger("click");
    expect(w.find(".jump").exists()).toBe(false);
    w.unmount();
  });
});
