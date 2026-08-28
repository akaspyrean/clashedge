// src/constants/groups.ts - 内置代理组的语义 ID 映射层
// mihomo 返回的组名是用户可见的真实名称（中文），业务逻辑（排序/类型判断）
// 不应直接比较中文字面量；此处提供「显示名 <-> 语义 ID」的双向映射。
// 语义 ID：manual=人工优选 / auto=自动优选 / ai=人工智能 / media=影音视听 /
//          proxy=扶梯出行（出口组）/ global=GLOBAL。
// 自定义订阅组不在映射表内，resolveGroupId 回退返回原名，天然兼容。

export type SemanticGroupId =
  | "manual"
  | "auto"
  | "ai"
  | "media"
  | "proxy"
  | "global";

/** 显示名 -> 语义 ID。 */
export const GROUP_NAME_TO_ID: Readonly<Record<string, SemanticGroupId>> = {
  "人工优选": "manual",
  "自动优选": "auto",
  "人工智能": "ai",
  "影音视听": "media",
  "扶梯出行": "proxy",
  "GLOBAL": "global",
};

/** 语义 ID -> 显示名。 */
export const GROUP_ID_TO_NAME: Readonly<
  Record<SemanticGroupId, string>
> = Object.fromEntries(
  Object.entries(GROUP_NAME_TO_ID).map(([name, id]) => [id, name])
) as Record<SemanticGroupId, string>;

/** 组名解析为语义 ID；未知名（自定义订阅组等）回退返回原名。 */
export function resolveGroupId(name: string): string {
  return GROUP_NAME_TO_ID[name] ?? name;
}
