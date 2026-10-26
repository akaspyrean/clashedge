// src/utils/latency.ts - 延迟分级（全局统一，阈值可在设置中调整）。无 Vue / Tauri 依赖。

export type LatencyKind = "good" | "mid" | "bad" | "timeout" | "testing" | "none";

export interface LatencyThresholds {
  /** ≤ good 为「良好」 */
  good: number;
  /** ≤ bad 为「一般」，高于为「较差」 */
  bad: number;
}

export const DEFAULT_LATENCY_THRESHOLDS: LatencyThresholds = { good: 200, bad: 500 };

export interface LatencyInput {
  ms?: number | null;
  testing?: boolean;
  timeout?: boolean;
  thresholds?: Partial<LatencyThresholds>;
}

/** 按 §3.4 分级：测速中 > 超时 > 未测 > 良好 / 一般 / 较差。ms ≤ 0 视为失败（mihomo 以 0 表示失败）。 */
export function classifyLatency({ ms, testing, timeout, thresholds }: LatencyInput): LatencyKind {
  if (testing) return "testing";
  if (timeout) return "timeout";
  if (ms === undefined || ms === null || !Number.isFinite(ms)) return "none";
  if (ms <= 0) return "timeout";
  const good = thresholds?.good ?? DEFAULT_LATENCY_THRESHOLDS.good;
  const bad = thresholds?.bad ?? DEFAULT_LATENCY_THRESHOLDS.bad;
  if (ms <= good) return "good";
  if (ms <= bad) return "mid";
  return "bad";
}

/**
 * 节点池是否整体不可用：**没有任何健康节点**（全部超时，或全部延迟高于「较差」阈值，
 * 或两者混合）。用于触发「节点数据 + 地理数据/规则」立即自愈更新。
 *
 * - 空列表不算退化（没有可判断的样本）；
 * - 健康 = 有限正数且 ≤ bad 阈值；0 / null / undefined / Infinity 均视为不可用。
 */
export function isNodePoolDegraded(
  delays: Array<number | null | undefined>,
  thresholds?: Partial<LatencyThresholds>,
): boolean {
  if (delays.length === 0) return false;
  const bad = thresholds?.bad ?? DEFAULT_LATENCY_THRESHOLDS.bad;
  return delays.every((ms) => !(typeof ms === "number" && Number.isFinite(ms) && ms > 0 && ms <= bad));
}
