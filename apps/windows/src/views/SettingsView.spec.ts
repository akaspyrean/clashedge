// SettingsView 冒烟测试：新模板（偏好行式）能挂载渲染，且关键行存在。
// 设置页此前无组件级测试；行式化改造后补上渲染冒烟，防止模板回归。
// 注意：theme.ts 在模块顶层调用 window.matchMedia，happy-dom 未提供，
// 因此先 stub 再动态导入被测组件。
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

import { createPinia, setActivePinia } from "pinia";
import ElementPlus from "element-plus";
import { createI18n } from "vue-i18n";

// SettingsView 在 setup 中调用 useI18n()，必须安装插件；空消息表下 t 返回 key，
// 与断言（按 key 匹配）一致。
const testI18n = createI18n({ legacy: false, locale: "zh-CN", messages: { "zh-CN": {} } });

/** 后端返回一份最小可用配置。 */
function mockBackend() {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "get_config")
      return Promise.resolve({
        mode: "rule",
        "system-proxy": false,
        locale: "zh-CN",
        "mixed-port": 7890,
        "allow-lan": false,
        ipv6: false,
        "log-level": "info",
        "geodata-mode": "manual",
        "geo-auto-update": false,
        "auto-update-subscription": false,
        "find-process-mode": "off",
        tun: { enable: false, stack: "mixed", "auto-route": true, "auto-detect-interface": true },
        advanced: { "geox-url": "", "geoip-url": "", "geosite-url": "" },
      });
    if (cmd === "get_supported_locales") return Promise.resolve(["zh-CN", "en-US"]);
    if (cmd === "get_geodata_status")
      return Promise.resolve({
        geoip: { exists: true, size: 1024 },
        geosite: { exists: true, size: 2048 },
      });
    if (cmd === "get_status") return Promise.resolve({ running: true, status: "running", version: null });
    if (cmd === "get_i18n_messages") return Promise.resolve({});
    return Promise.resolve(undefined);
  });
}

const mountOptions = {
  global: {
    plugins: [ElementPlus, testI18n],
    mocks: { $t: (k: string) => k },
  },
};

describe("SettingsView: 行式布局冒烟", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    invokeMock.mockReset();
    mockBackend();
    // theme.ts 模块顶层依赖 matchMedia（happy-dom 缺失），给一个最小实现。
    vi.stubGlobal("matchMedia", (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }));
    // SettingsView mounted 钩子用 ResizeObserver 观察 tabs 容器（happy-dom 缺失）。
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      }
    );
  });

  it("常规页渲染偏好行（语言/主题/系统代理），不再使用 el-form", async () => {
    const { default: SettingsView } = await import("@/views/SettingsView.vue");
    const wrapper = mount(SettingsView, mountOptions);
    await flushPromises();
    const html = wrapper.html();
    expect(html).toContain("pref-row");
    expect(html).toContain("pref-title");
    // 行式化之后不应再有表单式 label-width 布局
    expect(html).not.toContain("el-form-item");
    // 常规页关键行存在
    expect(wrapper.text()).toContain("settings.language");
    expect(wrapper.text()).toContain("settings.theme");
    expect(wrapper.text()).toContain("general.system_proxy");
  });

  it("高级页承接低频技术项（日志级别/地理数据/进程查找）", async () => {
    const { default: SettingsView } = await import("@/views/SettingsView.vue");
    const wrapper = mount(SettingsView, mountOptions);
    await flushPromises();
    const tabs = wrapper.findAll(".el-tabs__item");
    await tabs[tabs.length - 2]!.trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("general.log_level");
    expect(wrapper.text()).toContain("general.geodata_mode");
    expect(wrapper.text()).toContain("general.find_process_mode");
  });
});
