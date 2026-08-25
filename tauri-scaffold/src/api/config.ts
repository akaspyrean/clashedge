// src/api/config.ts
// 配置命令的类型化封装。
// 注意：后端 Config 模型使用 `#[serde(rename_all = "kebab-case")]`，
// `general` 与 `proxy` 均被 `#[serde(flatten)]` 展开到顶层，
// 所以 JSON 键是 kebab-case，且 external-controller / secret 位于顶层。
// `proxy_mode` 字段在模型上 `#[serde(rename = "mode", alias = "proxy-mode")]`，
// 因此 JSON 键是 `mode`（不是 0.8.5 旧版的 "proxy-mode"）；
// `system_proxy` 是应用级字段（托盘系统代理开关的真实状态），键为 "system-proxy"，
// 前端任何整包保存（update_config）都必须携带它，否则会被后端默认值覆盖。
// P0-3：`secret` 字段由后端脱敏为 "********"（真实密钥仅存后端，Rust 调用
// mihomo API 时直接读共享配置 Arc 做 Bearer 鉴权）。前端回传脱敏值时后端
// 保留现有真实密钥不轮换。

import { invoke } from "@tauri-apps/api/core";

export interface TunConfig {
  enable: boolean;
  stack: string;
  "auto-route": boolean;
  "auto-detect-interface": boolean;
  "interface-name": string | null;
}

export interface DnsConfig {
  enable: boolean;
  listen: string;
  ipv6: boolean;
  "enhanced-mode": string;
  "fake-ip-range": string;
  "fake-ip-filter": string[];
  "default-nameserver": string[];
  nameserver: string[];
}

export interface AdvancedConfig {
  "disable-commit-animation": boolean;
  "log-format": string;
  "explicit-proxy": boolean;
  "connect-timeout": number;
  "read-timeout": number;
  "write-timeout": number;
  "geox-url": string;
  "geoip-url": string;
  "geosite-url": string;
}

export interface ProfilesConfig {
  proxies: string[];
  "default-profile": string;
  "auto-group": string;
  "manual-group": string;
  "media-group": string;
  "ai-group": string;
}

export interface ClashConfig {
  "mixed-port": number;
  "allow-lan": boolean;
  /** allow-lan 的绑定地址："*" 或具体 IP；后端仅在 allow-lan 时写入 mihomo。 */
  "bind-address"?: string | null;
  /** 局域网访问 CIDR 白名单；非法值由后端兜底丢弃。 */
  "lan-allowed-ips"?: string[];
  "log-level": string;
  ipv6: boolean;
  "geodata-mode": string;
  "geo-auto-update": boolean;
  "find-process-mode": string;
  mode: string;
  profile: string;
  "system-proxy": boolean;
  "external-controller": string;
  secret: string;
  tun: TunConfig;
  dns: DnsConfig;
  advanced: AdvancedConfig;
  profiles: ProfilesConfig;
  "mixin-enabled": boolean;
  locale: string;
  "rule-providers": Record<string, unknown>;
}

export const configApi = {
  get: () => invoke<ClashConfig>("get_config"),
  update: (config: ClashConfig) => invoke<void>("update_config", { config }),
  /** 浅合并保存：仅提交 patch 中出现的顶层键（kebab-case），其余键保持后端现值。
   *  用于避免整包回传把用户停留期间其他入口（托盘等）修改的字段覆盖回去。 */
  updateFields: (patch: Record<string, unknown>) =>
    invoke<void>("update_config_fields", { patch }),
  reset: () => invoke<void>("reset_config"),
  export: () => invoke<string>("export_config"),
  import: (yaml: string) => invoke<void>("import_config", { yaml }),
  /** 弹出系统文件对话框选取 .yaml/.yml 并读取内容（选择与校验均在后端完成，
   *  WebView 不接触任意路径）。用户取消返回 null。 */
  pickImportFile: () => invoke<string | null>("pick_import_file"),
};
