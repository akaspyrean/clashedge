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

/** 速率拆成 [数值, 单位]，供大号数值 + 小号单位的排版："4.82 MB/s" → ["4.82", "MB/s"]。 */
export function splitRate(bytesPerSecond: number): [string, string] {
  const [value, unit] = formatBytes(bytesPerSecond).split(" ");
  return [value, `${unit}/s`];
}

/** 应用版本显示：发布版本补 "v" 前缀与 release tag 一致；"dev" 构建原样，已有 v 不重复加。 */
export function formatAppVersion(version: string): string {
  if (!version) return "";
  if (version === "dev") return "dev";
  return /^v/i.test(version) ? version : `v${version}`;
}

/** 遮蔽占位（与后端 redact_subscribe_url 一致；固定长度，不泄露原值长度）。 */
const URL_MASK = "••••••••";

/** 形似 token 的路径段：≥8 字符、仅 `[A-Za-z0-9_-]` 且含数字（与后端同规则）。 */
function isTokenLikeSegment(seg: string): boolean {
  return seg.length >= 8 && /^[A-Za-z0-9_-]+$/.test(seg) && /\d/.test(seg);
}

/** 订阅地址显示脱敏（与后端 redact_subscribe_url 同策略，前端幂等兜底）：
 *  保留 scheme://host[:port]、路径结构与 query 键名；所有疑似 token 的路径段与
 *  query 值一律换成 ••••••••。解析失败返回 "…"（不透出原文）。 */
export function maskUrl(url: string): string {
  try {
    const u = new URL(url);
    // URL 会把路径里的 • 规范化为 %E2%80%A2…；解码回来保证幂等（已遮蔽值不再二次处理）
    let pathname = u.pathname;
    try {
      pathname = decodeURIComponent(pathname);
    } catch {
      /* 保留原样 */
    }
    let path = "";
    if (pathname !== "/" && pathname !== "") {
      const segs = pathname.replace(/^\/+/, "").split("/");
      // 遮蔽所有形似 token 的路径段（≥8 字符、仅字母数字下划线连字符且含数字）
      for (let i = 0; i < segs.length; i++) {
        if (isTokenLikeSegment(segs[i])) segs[i] = URL_MASK;
      }
      path = `/${segs.join("/")}`;
    }
    const query = u.search ? `?${[...u.searchParams.keys()].map((k) => `${k}=${URL_MASK}`).join("&")}` : "";
    return `${u.protocol}//${u.host}${path}${query}`;
  } catch {
    return "…";
  }
}

/** 常用地区名到国旗 Emoji 的映射 */
const REGION_FLAGS: Record<string, string> = {
  香港: "🇭🇰",
  日本: "🇯🇵",
  美国: "🇺🇸",
  新加坡: "🇸🇬",
  台湾: "🇹🇼",
  英国: "🇬🇧",
  德国: "🇩🇪",
  韩国: "🇰🇷",
  荷兰: "🇳🇱",
  加拿大: "🇨🇦",
  澳大利亚: "🇦🇺",
  法国: "🇫🇷",
  俄罗斯: "🇷🇺",
  土耳其: "🇹🇷",
  印度: "🇮🇳",
  巴西: "🇧🇷",
  阿根廷: "🇦🇷",
  马来西亚: "🇲🇾",
  泰国: "🇹🇭",
  越南: "🇻🇳",
  菲律宾: "🇵🇭",
  印度尼西亚: "🇮🇩",
};

/** 常用地区名到 ISO 国家两字码（供 Flyme 风格精致微标展示，彻底避免字体错位） */
const REGION_CODES: Record<string, string> = {
  香港: "HK",
  日本: "JP",
  美国: "US",
  新加坡: "SG",
  台湾: "TW",
  英国: "GB",
  德国: "DE",
  韩国: "KR",
  荷兰: "NL",
  加拿大: "CA",
  澳大利亚: "AU",
  法国: "FR",
  俄罗斯: "RU",
  土耳其: "TR",
  印度: "IN",
  巴西: "BR",
  阿根廷: "AR",
  马来西亚: "MY",
  泰国: "TH",
  越南: "VN",
  菲律宾: "PH",
  印度尼西亚: "ID",
};

export function getRegionFlag(region?: string | null): string | undefined {
  if (!region) return undefined;
  for (const [key, flag] of Object.entries(REGION_FLAGS)) {
    if (region.includes(key)) return flag;
  }
  return undefined;
}

export function getRegionCode(region?: string | null): string | undefined {
  if (!region) return undefined;
  for (const [key, code] of Object.entries(REGION_CODES)) {
    if (region.includes(key)) return code;
  }
  return undefined;
}

/** 智能清洗机场原始长节点名（如 JMS-1416430@c37s1.portablesub.net -> c37s1），
 *  并组合易读名称与国家识别码。
 */
export function formatNodeDisplay(rawName: string, region?: string | null): {
  displayName: string;
  flag?: string;
  regionCode?: string;
  cleanName: string;
} {
  const flag = getRegionFlag(region);
  const regionCode = getRegionCode(region);

  // 1. 去除形如 JMS-1416430@、任意 [A-Za-z0-9_-]+@ 的账号长前缀
  let clean = rawName.replace(/^[A-Za-z0-9_.-]+@/, "").trim();

  // 2. 去除长域名后缀（如 .portablesub.net、.sub.xyz 等）
  clean = clean.replace(/\.(portablesub|sub|node|server|v2ray|shadowsocks|vpn)\.[a-zA-Z0-9.-]+$/i, "");
  // 如果剩余部分仍带有域名尾缀（如 c37s1.com / c37s1.net），去除常见顶级域
  clean = clean.replace(/\.(com|net|org|xyz|top|io|me|info|vip|club)$/i, "");

  // 3. 去除类似 "【VIP专线】" 或头部方括号广告
  clean = clean.replace(/^\[[^\]]+\]/, "").trim();
  clean = clean.replace(/^【[^】]+】/, "").trim();

  // 如果清洗后为空，回退原名
  if (!clean) clean = rawName;

  // 4. 生成主展示名：若已经包含地区词（如 "香港 01"），直接保留；
  // 否则若有外部识别出的地区（如 "美国"），组合成 "美国 · c37s1"
  let displayName = clean;
  if (region && !clean.includes(region)) {
    displayName = `${region} · ${clean}`;
  }

  return {
    displayName,
    flag,
    regionCode,
    cleanName: clean,
  };
}
