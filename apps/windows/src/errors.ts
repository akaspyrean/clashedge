// src/errors.ts - 面向用户的错误文案。
//
// 底层错误串（reqwest / IO / 端口占用）直接 toast 出来对用户没有可操作性。
// 这里把最常见的几类映射为 i18n 的人话提示；识别不出的错误原样透出
// （诚实优先：宁可显示原始串，也不给错误信息安一个错误的解释）。

import { t } from "@/i18n";

/** 把捕获到的错误转成适合 toast 的一句话。 */
export function friendlyError(e: unknown): string {
  const raw = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  const lower = raw.toLowerCase();
  if (
    lower.includes("error sending request") ||
    lower.includes("timed out") ||
    lower.includes("timeout") ||
    lower.includes("connection refused") ||
    lower.includes("dns error") ||
    lower.includes("unreachable")
  ) {
    return t("errors.network");
  }
  if (
    lower.includes("bind") ||
    lower.includes("port") ||
    lower.includes("address already in use")
  ) {
    return t("errors.port_busy");
  }
  return raw;
}
