// src/utils/format.ts - 视图共用的纯格式化函数（可单测，无 Vue / Tauri 依赖）。

/** B / KB / MB / GB / TB，大数少小数位（<10: 2 位，<100: 1 位，其余 0 位）。 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
  const value = bytes / Math.pow(1024, i);
  const digits = i === 0 ? 0 : value >= 100 ? 0 : value >= 10 ? 1 : 2;
  return `${value.toFixed(digits)} ${units[i]}`;
}

export interface SubscriptionTraffic {
  /** 已用字节（upload + download） */
  used: number;
  /** 总额字节；0 / 缺省 = 不限量 */
  total: number;
  /** 到期 unix 秒；0 = 长期有效；undefined = 响应头未携带 */
  expire?: number;
}

/** 解析 `Subscription-Userinfo`（`upload=..; download=..; total=..; expire=..`）。
 *  无法解析 / 无任何流量字段返回 null。 */
export function parseUserinfo(info?: string | null): SubscriptionTraffic | null {
  if (!info) return null;
  const m: Record<string, number> = {};
  for (const part of info.split(";")) {
    const [k, v] = part.split("=");
    if (!k || v === undefined) continue;
    const n = Number(v.trim());
    if (Number.isFinite(n)) m[k.trim().toLowerCase()] = n;
  }
  if (!("total" in m) && !("upload" in m) && !("download" in m)) return null;
  return {
    used: (m.upload ?? 0) + (m.download ?? 0),
    total: m.total ?? 0,
    expire: "expire" in m ? m.expire : undefined,
  };
}

/** CIDR 形态校验：IPv4 x.x.x.x/n（0-32）；含 ":" 的 IPv6 前缀仅做形态放行。 */
export function isValidCidr(entry: string): boolean {
  if (entry.includes(":")) return true;
  const m = entry.match(/^(\d{1,3}(?:\.\d{1,3}){3})\/(\d{1,2})$/);
  if (!m) return false;
  if (Number(m[2]) > 32) return false;
  return m[1].split(".").every((oct) => Number(oct) <= 255);
}

/** 速率（字节/秒）→ "4.82 MB/s"。 */
export function formatRate(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}
