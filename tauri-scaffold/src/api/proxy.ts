// src/api/proxy.ts
// 代理命令的类型化封装（系统代理 / TUN / 代理模式 / 代理组）

import { invoke } from "@tauri-apps/api/core";

export interface ProxyGroup {
  name: string;
  type: string;
  now: string;
  all: string[];
}

export interface DelayResult {
  group: string;
  delay: number | null;
  message?: string;
}

export const proxyApi = {
  setSystemProxy: (enable: boolean) => invoke<void>("set_system_proxy", { enable }),
  setTunMode: (enable: boolean) => invoke<void>("set_tun_mode", { enable }),
  setProxyMode: (mode: string) => invoke<void>("set_proxy_mode", { mode }),
  testLatency: (group: string, url?: string) =>
    invoke<DelayResult[]>("test_proxy_latency", { group, url }),
  getGroups: () => invoke<ProxyGroup[]>("get_proxy_groups"),
  select: (group: string, proxy: string) =>
    invoke<void>("select_proxy_group", { group, proxy }),
};
