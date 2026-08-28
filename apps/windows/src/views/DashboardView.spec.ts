import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { createPinia, setActivePinia } from "pinia";
import ElementPlus from "element-plus";
import DashboardView from "@/views/DashboardView.vue";
import { useConfigStore } from "@/stores/config";
import type { ClashConfig } from "@/api/config";

function baseConfig(extra: Partial<Record<string, unknown>> = {}): ClashConfig {
  return {
    "mixed-port": 7890,
    "allow-lan": false,
    "log-level": "info",
    ipv6: false,
    "geodata-mode": "manual",
    "geo-auto-update": false,
    "auto-update-subscription": true,
    "find-process-mode": "off",
    mode: "rule",
    profile: "DIRECT",
    "system-proxy": false,
    "external-controller": "127.0.0.1:9090",
    secret: "********",
    tun: { enable: false, stack: "gvisor", "auto-route": true, "auto-detect-interface": true, "interface-name": null },
    dns: { enable: false, listen: "0.0.0.0:1053", ipv6: false, "enhanced-mode": "fake-ip", "fake-ip-range": "198.18.0.1/16", "fake-ip-filter": [], "default-nameserver": [], nameserver: [] },
    advanced: { "disable-commit-animation": false, "log-format": "text", "explicit-proxy": true, "connect-timeout": 5, "read-timeout": 30, "write-timeout": 30, "geox-url": "", "geoip-url": "", "geosite-url": "" },
    profiles: { proxies: [], "default-profile": "DIRECT", "auto-group": "Auto", "manual-group": "Manual", "media-group": "Media", "ai-group": "AI" },
    "mixin-enabled": false,
    locale: "zh-CN",
    "rule-providers": {},
    ...extra,
  };
}

/** 模拟「后端当前 system-proxy 值」为 v，get_config 返回它。 */
function mockBackendSystemProxy(v: boolean) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "get_config") return Promise.resolve(baseConfig({ "system-proxy": v }));
    if (cmd === "get_status") return Promise.resolve({ running: true, status: "running", version: null });
    if (cmd === "get_proxy_groups") return Promise.resolve([{ name: "GLOBAL", type: "Selector", now: "DIRECT", all: ["DIRECT"] }]);
    if (cmd === "set_system_proxy") return Promise.resolve(null);
    return Promise.resolve(undefined);
  });
}

const mountOptions = {
  global: {
    plugins: [ElementPlus],
    mocks: { $t: (k: string) => k },
  },
};

describe("Dashboard system-proxy ↔ 后端状态联动", () => {
  beforeEach(async () => {
    setActivePinia(createPinia());
    invokeMock.mockReset();
    // 模拟 App.vue 启动：先加载配置，再 mount Dashboard
    mockBackendSystemProxy(false);
    await useConfigStore().load();
  });

  it("打开页面后开关反映后端真实状态", async () => {
    mockBackendSystemProxy(false);
    const wrapper = mount(DashboardView, mountOptions);
    await flushPromises();
    const cfg = useConfigStore();
    expect(cfg.systemProxy).toBe(false);
    // el-switch input 的 checked 应为 false
    const input = wrapper.find(".set-row input[type=checkbox]").element as HTMLInputElement;
    expect(input.checked).toBe(false);
  });

  it("模拟托盘第一次开启 → load 后 store 与开关 UI 都切换到开", async () => {
    mockBackendSystemProxy(false);
    const wrapper = mount(DashboardView, mountOptions);
    await flushPromises();
    const cfg = useConfigStore();

    // 托盘开启：后端变为 true，main.ts 会调 configStore.load()
    mockBackendSystemProxy(true);
    await cfg.load();
    await flushPromises();
    expect(cfg.systemProxy).toBe(true);
    const input = wrapper.find(".set-row input[type=checkbox]").element as HTMLInputElement;
    expect(input.checked).toBe(true);
  });

  it("模拟托盘第二次关闭 → load 后 store 与开关 UI 都切回关（第二次刷新不失效）", async () => {
    mockBackendSystemProxy(true);
    const wrapper = mount(DashboardView, mountOptions);
    await flushPromises();
    const cfg = useConfigStore();
    await cfg.load();
    expect(cfg.systemProxy).toBe(true);

    // 托盘第二次切换：关闭
    mockBackendSystemProxy(false);
    await cfg.load();
    await flushPromises();
    expect(cfg.systemProxy).toBe(false);
    const input = wrapper.find(".set-row input[type=checkbox]").element as HTMLInputElement;
    expect(input.checked).toBe(false);
  });

  it("用户点击开关 → set_system_proxy 调用，且本地 store 立即同步", async () => {
    mockBackendSystemProxy(false);
    const wrapper = mount(DashboardView, mountOptions);
    await flushPromises();

    await wrapper.find(".set-row input[type=checkbox]").setValue(true);
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("set_system_proxy", { enable: true });
    const cfg = useConfigStore();
    expect(cfg.systemProxy).toBe(true);
  });
});