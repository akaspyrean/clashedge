// src/utils/region.ts - 按节点名推断地区（仅用于代理页的筛选芯片，无任何网络请求）。
// 机场节点命名五花八门：同时匹配中文名、英文名和两字母代码；代码两侧不能紧邻字母，
// 避免 "USB" 被当成 US。匹配不上一律归入 other。

export type RegionId = "hk" | "tw" | "jp" | "kr" | "sg" | "us" | "uk" | "de" | "other";

const code = (c: string) => `(?<![A-Za-z])${c}(?![A-Za-z])`;

const RULES: ReadonlyArray<readonly [Exclude<RegionId, "other">, RegExp]> = [
  ["hk", new RegExp(`香港|Hong ?Kong|${code("HK")}`, "i")],
  ["tw", new RegExp(`台湾|台灣|Taiwan|${code("TW")}`, "i")],
  ["jp", new RegExp(`日本|东京|大阪|Japan|Tokyo|Osaka|${code("JP")}`, "i")],
  ["kr", new RegExp(`韩国|韓國|首尔|Korea|Seoul|${code("KR")}`, "i")],
  ["sg", new RegExp(`新加坡|狮城|Singapore|${code("SG")}`, "i")],
  ["us", new RegExp(`美国|美國|洛杉矶|圣何塞|纽约|United States|America|${code("USA?")}`, "i")],
  ["uk", new RegExp(`英国|英國|伦敦|United Kingdom|London|${code("UK")}|${code("GB")}`, "i")],
  ["de", new RegExp(`德国|德國|法兰克福|Germany|Frankfurt|${code("DE")}`, "i")],
];

export function regionOf(name: string): RegionId {
  for (const [id, re] of RULES) if (re.test(name)) return id;
  return "other";
}
