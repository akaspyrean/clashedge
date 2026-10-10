import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { createPinia, setActivePinia } from "pinia";
import ElementPlus from "element-plus";
import { testI18n } from "@/test/harness";
import ProxiesView from "@/views/ProxiesView.vue";
import { useProxyStore } from "@/stores/proxy";
import type { ProxyGroup } from "@/api/proxy";

const groups: ProxyGroup[] = [
  { name: "GLOBAL", type: "Selector", now: "人工优选", all: ["DIRECT", "REJECT", "人工优选", "自动优选"] },
  { name: "扶梯出行", type: "Selector", now: "DIRECT", all: ["DIRECT", "自动优选"] },
  { name: "人工优选", type: "Selector", now: "Node1", all: ["DIRECT", "Node1", "Node2"] },
  { name: "自动优选", type: "URLTest", now: "Node1", all: ["Node1", "Node2"] },
];

/** 后端返回 rule 模式 + 运行中 + 上述代理组。 */
function mockBackend(mode = "rule") {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "get_config") return Promise.resolve({ mode, "system-proxy": false });
    if (cmd === "get_status") return Promise.resolve({ running: true, status: "running", version: null });
    if (cmd === "get_proxy_groups") return Promise.resolve(groups);
    return Promise.resolve(undefined);
  });
}

const mountOptions = {
  global: {
    plugins: [ElementPlus, testI18n()],
    mocks: { $t: (k: string) => k },
  },
};

describe("ProxiesView: 组可见性联动", () => {
  beforeEach(async () => {
    setActivePinia(createPinia());
    invokeMock.mockReset();
    mockBackend("rule");
    await useProxyStore().loadGroups();
  });

  it("rule 模式下隐藏 GLOBAL、显示真实组（扶梯出行/人工优选/自动优选）", async () => {
    const wrapper = mount(ProxiesView, mountOptions);
    await flushPromises();
    const store = useProxyStore();
    expect(store.groups.length).toBeGreaterThan(0);
    // GLOBAL 不出现在列表，人工优选/自动优选出现
    const text = wrapper.text();
    expect(text).not.toContain("GLOBAL");
    expect(text).toContain("人工优选");
    expect(text).toContain("自动优选");
  });

  it("无组时渲染空态提示（core 未运行场景）", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_config") return Promise.resolve({ mode: "rule", "system-proxy": false });
      if (cmd === "get_status") return Promise.resolve({ running: false, status: "stopped", version: null });
      if (cmd === "get_proxy_groups") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    await useProxyStore().loadGroups();
    const wrapper = mount(ProxiesView, mountOptions);
    await flushPromises();
    expect(wrapper.find(".proxy-empty").exists()).toBe(true);
  });
});

describe("ProxiesView: 主从双栏", () => {
  async function mountView(mode = "rule") {
    setActivePinia(createPinia());
    invokeMock.mockReset();
    mockBackend(mode);
    const { useConfigStore } = await import("@/stores/config");
    await useConfigStore().load();
    await useProxyStore().loadGroups();
    const w = mount(ProxiesView, mountOptions);
    await flushPromises();
    return w;
  }

  /** 左栏新顺序：叶子组（人工优选/自动优选）在前，路由组（扶梯出行等）在后 */
  it("组名按叶子→路由分段排列；叶子组标注手动或测速", async () => {
    const w = await mountView();
    const names = w.findAll(".group-name").map((n) => n.text());
    expect(names).toEqual(["人工优选", "自动优选", "扶梯出行"]);
    const types = w.findAll(".group-type").map((n) => n.text());
    expect(types).toEqual(["proxies.type_manual", "proxies.type_auto"]);
  });

  it("默认选中人工优选组（叶子组优先）；叶子组屏蔽 DIRECT", async () => {
    const w = await mountView();
    // 默认就是人工优选（第一个叶子组），无需点击
    expect(w.find(".detail-title").text()).toBe("人工优选");
    expect(w.findAll(".ce-tile__name").map((n) => n.text())).toEqual(["Node1", "Node2"]);
  });

  it("点节点格调用 select_proxy_group", async () => {
    const w = await mountView();
    // 默认已选中人工优选组，直接点第二个节点
    await w.findAll(".ce-tile__main")[1].trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("select_proxy_group", { group: "人工优选", proxy: "Node2" });
  });

  it("自动优选组节点格只读并提示由测速自动选择", async () => {
    const w = await mountView();
    // 自动优选是叶子组段的第 2 个按钮
    const rows = w.findAll(".group-row");
    const autoRow = rows.find((r) => r.text().includes("自动优选"));
    await autoRow!.trigger("click");
    expect(w.find(".note").text()).toBe("proxies.auto_readonly");
    await w.findAll(".ce-tile__main")[1].trigger("click");
    await flushPromises();
    expect(invokeMock).not.toHaveBeenCalledWith("select_proxy_group", expect.anything());
  });

  it("搜索框按名称过滤节点，无结果时给出说明", async () => {
    const w = await mountView();
    // 默认选中人工优选组，直接搜索
    await w.find(".search-box input").setValue("node2");
    expect(w.findAll(".ce-tile__name").map((n) => n.text())).toEqual(["Node2"]);
    await w.find(".search-box input").setValue("zzz");
    expect(w.find(".ce-tile").exists()).toBe(false);
    expect(w.find(".note").text()).toBe("proxies.no_match");
  });

  it("节点格已按延迟排序（已测升序在前）", async () => {
    const w = await mountView();
    const store = useProxyStore();
    store.nodeDelays["Node1"] = 300;
    store.nodeDelays["Node2"] = 50;
    await flushPromises();
    // 始终按延迟排序：Node2 (50ms) 在 Node1 (300ms) 前
    expect(w.findAll(".ce-tile__name").map((n) => n.text())).toEqual(["Node2", "Node1"]);
  });

  it("direct 模式显示空状态说明", async () => {
    const w = await mountView("direct");
    expect(w.find(".proxy-empty").text()).toContain("proxies.direct_hint");
    expect(w.find(".group-list").exists()).toBe(false);
  });

  it("global 模式只显示 GLOBAL", async () => {
    const w = await mountView("global");
    expect(w.findAll(".group-name").map((n) => n.text())).toEqual(["GLOBAL"]);
  });
});
