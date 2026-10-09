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

  it("五个分组齐全且顺序为 常规 / 代理 / TUN / 高级 / 关于", async () => {
    const { default: SettingsView } = await import("@/views/SettingsView.vue");
    const wrapper = mount(SettingsView, mountOptions);
    await flushPromises();
    expect(wrapper.findAll(".group-title").map((h) => h.text())).toEqual([
      "settings.tabs.general",
      "settings.tabs.proxy",
      "settings.tabs.tun",
      "settings.tabs.advanced",
      "settings.tabs.about",
    ]);
  });

  it("整页只有一个主按钮（保存）", async () => {
    const { default: SettingsView } = await import("@/views/SettingsView.vue");
    const wrapper = mount(SettingsView, mountOptions);
    await flushPromises();
    const primaries = wrapper.findAll(".el-button--primary");
    expect(primaries).toHaveLength(1);
    expect(primaries[0].text()).toBe("common.save");
  });

  it("搜索设置：命中的行高亮并报告数量，清空后恢复", async () => {
    const { default: SettingsView } = await import("@/views/SettingsView.vue");
    const wrapper = mount(SettingsView, { ...mountOptions, attachTo: document.body });
    await flushPromises();
    const input = wrapper.find(".search-box input");
    await input.setValue("general.mixed_port");
    await flushPromises();
    const hits = wrapper.findAll(".pref-row.pref-hit");
    expect(hits).toHaveLength(1);
    expect(hits[0].text()).toContain("general.mixed_port");
    expect(wrapper.find(".found").text()).toBe("settings.search_count");
    await input.setValue("zzzz-nothing");
    await flushPromises();
    expect(wrapper.findAll(".pref-hit")).toHaveLength(0);
    expect(wrapper.find(".found").text()).toBe("settings.search_none");
    await input.setValue("");
    await flushPromises();
    expect(wrapper.find(".found").exists()).toBe(false);
    wrapper.unmount();
  });

  it("高级页承接低频技术项（日志级别/地理数据/进程查找）", async () => {
    const { default: SettingsView } = await import("@/views/SettingsView.vue");
    const wrapper = mount(SettingsView, mountOptions);
    await flushPromises();
    // 单页分组：不再有标签页，高级项与常规项同页可见。
    expect(wrapper.find(".el-tabs").exists()).toBe(false);
    expect(wrapper.text()).toContain("general.log_level");
    expect(wrapper.text()).toContain("general.geodata_mode");
    expect(wrapper.text()).toContain("general.find_process_mode");
  });
});
