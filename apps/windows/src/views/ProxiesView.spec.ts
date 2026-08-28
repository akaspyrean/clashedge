import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { createPinia, setActivePinia } from "pinia";
import ElementPlus from "element-plus";
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
    plugins: [ElementPlus],
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
