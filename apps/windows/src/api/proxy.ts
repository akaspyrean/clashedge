// src/api/proxy.ts
// 代理命令的类型化封装（系统代理 / TUN / 代理模式 / 代理组）

import { invoke } from "@tauri-apps/api/core";

export interface ProxyGroup {
  name: string;
  type: string;
  now: string;
  all: string[];
  /** 节点名 → 地区（后端按节点名推断；识别不到的节点不会出现在表里）。 */
  regions?: Record<string, string>;
}

export interface DelayResult {
  group: string;
  delay: number | null;
  message?: string;
}

export const proxyApi = {
  setSystemProxy: (enable: boolean) => invoke<void>("set_system_proxy", { enable }),
  setTunMode: (enable: boolean) => invoke<void>("set_tun_mode", { enable }),
  /** 以管理员身份重启应用以开启 TUN（UAC 弹窗；授权后旧实例退出，通常不会 resolve）。 */
  relaunchForTun: () => invoke<void>("relaunch_for_tun"),
  setProxyMode: (mode: string) => invoke<void>("set_proxy_mode", { mode }),
  testLatency: (group: string, url?: string) =>
    invoke<DelayResult[]>("test_proxy_latency", { group, url }),
  getGroups: () => invoke<ProxyGroup[]>("get_proxy_groups"),
  select: (group: string, proxy: string) =>
    invoke<void>("select_proxy_group", { group, proxy }),
  /**
   * 节点池整体不可用（全部超时 / 全部异常慢）→ 立即触发「节点数据 + 地理数据/规则」
   * 自愈更新。后端按两个自动更新开关分别执行，并自带 10 分钟冷却限流。
   */
  triggerAutoRecovery: () =>
    invoke<{
      triggered: boolean;
      refreshed: number;
      geo_updated: boolean;
      reason: string;
    }>("trigger_auto_recovery"),
};
