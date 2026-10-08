// 设置页无障碍回归：每个控件都必须有可访问名称（来自 PrefRow 的标题），
// 每个偏好行都是被标题命名的 group。新增控件忘了接 `:aria-label="label"` 时这里会红。
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));

import { mountApp, stubBrowserApis } from "@/test/harness";

const config = {
  mode: "rule",
  "system-proxy": false,
  locale: "zh-CN",
  "mixed-port": 7890,
  "allow-lan": true,
  ipv6: false,
  sniffer: false,
  "log-level": "info",
  "geodata-mode": "manual",
  "geo-auto-update": false,
  "auto-update-subscription": false,
  "auto-check-update": true,
  "find-process-mode": "off",
  tun: { enable: false, stack: "mixed", "auto-route": true, "auto-detect-interface": true },
  advanced: { "geoip-url": "", "geosite-url": "" },
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "get_config") return Promise.resolve(config);
    if (cmd === "get_supported_locales") return Promise.resolve(["zh-CN", "en-US"]);
    if (cmd === "get_geodata_status")
      return Promise.resolve({ geoip: { exists: true, size: 1, url: "" }, geosite: { exists: true, size: 1, url: "" } });
    if (cmd === "get_config_degraded") return Promise.resolve({ degraded: false, backup_file: null, message: "" });
    if (cmd === "get_autostart") return Promise.resolve(false);
    if (cmd === "get_status") return Promise.resolve({ running: true, status: "running", version: null });
    return Promise.resolve(undefined);
  });
  stubBrowserApis(vi);
});

describe("SettingsView accessibility", () => {
  it("every switch, select and number input has an accessible name", async () => {
    const { default: SettingsView } = await import("@/views/SettingsView.vue");
    const w = mountApp(SettingsView);
    await flushPromises();

    const switches = w.findAll("input[role=switch]");
    expect(switches.length).toBeGreaterThan(8);
    for (const s of switches) {
      expect(s.attributes("aria-label"), s.html()).toBeTruthy();
    }

    // el-select / el-input-number 的可聚焦元素：input 必须有 aria-label
    const named = w.findAll(".el-select input, .el-input-number input");
    expect(named.length).toBeGreaterThan(4);
    for (const i of named) {
      expect(i.attributes("aria-label"), i.html()).toBeTruthy();
    }
  });

  it("every preference row is a group named by its own title", async () => {
    const { default: SettingsView } = await import("@/views/SettingsView.vue");
    const w = mountApp(SettingsView);
    await flushPromises();

    const rows = w.findAll(".pref-row");
    expect(rows.length).toBeGreaterThan(15);
    for (const row of rows) {
      expect(row.attributes("role")).toBe("group");
      const id = row.attributes("aria-labelledby")!;
      expect(id).toBeTruthy();
      expect(row.find(`[id="${id}"]`).exists()).toBe(true);
    }
    // id 唯一，否则 aria-labelledby 会指到别的行
    const ids = rows.map((r) => r.attributes("aria-labelledby"));
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("the invalid LAN CIDR warning is announced as an alert", async () => {
    const { default: SettingsView } = await import("@/views/SettingsView.vue");
    const w = mountApp(SettingsView);
    await flushPromises();
    const ta = w.find("#lan-allowed-ips");
    expect(ta.exists()).toBe(true);
    await ta.setValue("not-a-cidr");
    await flushPromises();
    expect(w.find(".lan-ips-warning[role=alert]").exists()).toBe(true);
    expect(ta.attributes("aria-invalid")).toBe("true");
  });
});
