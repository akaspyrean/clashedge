import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock @tauri-apps/api/core 的 invoke：store 的 save() / patch() 走 invoke
// 触发后端命令；测试只关心前端 store 状态机，不验证后端行为。
const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { setActivePinia, createPinia } from "pinia";
import { useConfigStore } from "@/stores/config";
import type { ClashConfig } from "@/api/config";

function baseConfig(): ClashConfig {
  return {
    "mixed-port": 7890,
    "allow-lan": false,
    "log-level": "info",
    ipv6: false,
    "geodata-mode": "manual",
    "geo-auto-update": false,
    "find-process-mode": "off",
    mode: "rule",
    profile: "DIRECT",
    "system-proxy": false,
    "external-controller": "127.0.0.1:9090",
    secret: "********",
    tun: {
      enable: false,
      stack: "gvisor",
      "auto-route": true,
      "auto-detect-interface": true,
      "interface-name": null,
    },
    dns: {
      enable: false,
      listen: "0.0.0.0:1053",
      ipv6: false,
      "enhanced-mode": "fake-ip",
      "fake-ip-range": "198.18.0.1/16",
      "fake-ip-filter": [],
      "default-nameserver": [],
      nameserver: [],
    },
    advanced: {
      "disable-commit-animation": false,
      "log-format": "text",
      "explicit-proxy": true,
      "connect-timeout": 5,
      "read-timeout": 30,
      "write-timeout": 30,
      "geox-url": "",
      "geoip-url": "",
      "geosite-url": "",
    },
    profiles: {
      proxies: [],
      "default-profile": "DIRECT",
      "auto-group": "Auto",
      "manual-group": "Manual",
      "media-group": "Media",
      "ai-group": "AI",
    },
    "mixin-enabled": false,
    locale: "zh-CN",
    "rule-providers": {},
  };
}

/** 直接准备 store 的 config + baseline，绕过 load()（load 调 invoke 且
 *  structuredClone 对 Pinia reactive proxy 在 jsdom 下会报 DataCloneError——
 *  生产环境无此问题，但测试需要可重复）。 */
function primeStore(store: ReturnType<typeof useConfigStore>, cfg: ClashConfig) {
  store.config = JSON.parse(JSON.stringify(cfg));
  store.baseline = JSON.parse(JSON.stringify(cfg));
}

describe("config store / diffFromBaseline (P0-3 深比较)", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    invokeMock.mockReset();
  });

  it("load 后 baseline 与 config 内容相同 → diff 为空", () => {
    const store = useConfigStore();
    primeStore(store, baseConfig());

    const diff = store.diffFromBaseline();
    expect(Object.keys(diff)).toHaveLength(0);
  });

  it("改一个 primitive 字段 → 只该键进 diff（不污染嵌套对象）", () => {
    const store = useConfigStore();
    primeStore(store, baseConfig());

    // 用户只在 Settings 改了 mixed-port，没碰 tun/dns
    if (store.config) store.config["mixed-port"] = 7891;

    const diff = store.diffFromBaseline();
    expect(Object.keys(diff)).toEqual(["mixed-port"]);
    expect(diff["mixed-port"]).toBe(7891);
  });

  it("改 tun.enable → 只 tun 进 diff，dns 不进", () => {
    const store = useConfigStore();
    primeStore(store, baseConfig());

    if (store.config && store.config.tun) store.config.tun.enable = true;

    const diff = store.diffFromBaseline();
    expect(Object.keys(diff)).toEqual(["tun"]);
    expect((diff.tun as { enable: boolean }).enable).toBe(true);
  });

  it("未改 tun 内容时 tun 不进 diff（核心回归：浅引用比较会让 tun 恒进 diff）",
    () => {
    const store = useConfigStore();
    primeStore(store, baseConfig());

    // 用户只改了一个无关字段
    if (store.config) store.config["allow-lan"] = true;

    const diff = store.diffFromBaseline();
    // 必须只有 allow-lan，tun/dns 等嵌套对象不能因引用比较被误判为已修改
    expect(Object.keys(diff)).toEqual(["allow-lan"]);
    expect(diff.tun).toBeUndefined();
    expect(diff.dns).toBeUndefined();
  });

  it("把 tun 改回原值后 tun 不进 diff（深比较感知内容相同）", () => {
    const store = useConfigStore();
    primeStore(store, baseConfig());

    // 改了又改回
    if (store.config && store.config.tun) {
      store.config.tun.enable = true;
      store.config.tun.enable = false;
    }

    const diff = store.diffFromBaseline();
    expect(diff.tun).toBeUndefined();
  });

  it("save() 无 diff 时不调 updateFields", async () => {
    const store = useConfigStore();
    primeStore(store, baseConfig());

    invokeMock.mockClear();
    await store.save();
    expect(invokeMock).not.toHaveBeenCalled();
  });
});

