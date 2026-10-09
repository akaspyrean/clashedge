// src/api/rules.ts
// 规则命令的类型化封装（get_rules：只读，Rust 侧已裁剪为 type / payload / proxy）。

import { invoke } from "@tauri-apps/api/core";

export interface RuleInfo {
  /** 规则类型：DomainSuffix / GeoSite / IPCIDR / Match … */
  type: string;
  /** 匹配内容；Match 规则为空字符串 */
  payload: string;
  /** 策略：代理组名 / DIRECT / REJECT */
  proxy: string;
}

export const rulesApi = {
  list: () => invoke<RuleInfo[]>("get_rules"),
};
