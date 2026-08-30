// src/i18n/index.ts
// 前端国际化入口。
// 单一来源：后端 resources/i18n/*.yaml（tray 菜单与前端共用）。
// 后端通过 get_i18n_messages 返回扁平键表，此处还原为嵌套对象供 vue-i18n 使用
// （vue-i18n 11 不依赖 flatJson，手动 unflatten 更稳妥）。

import { createI18n, type I18n } from "vue-i18n";
import { configApi } from "@/api/config";
import { utilApi } from "@/api/util";

export const DEFAULT_LOCALE = "zh-CN";

/** 当前生效语言（供 main.ts 初始化 Element Plus 语言包等非响应式消费）。 */
export let currentLocale = DEFAULT_LOCALE;

/** 嵌套消息树：叶子是翻译文本，中间层是对象。与 vue-i18n 的 LocaleMessage 结构兼容。 */
interface MessageTree {
  [key: string]: string | MessageTree;
}

/** 将 {"a.b.c": "值"} 的扁平表还原为嵌套对象。
 *  中间节点与根对象使用无原型对象（Object.create(null)），并显式跳过
 *  __proto__ / constructor / prototype 键，防止后端消息键被恶意构造时原型污染。 */
const DANGEROUS_KEYS = new Set(["__proto__", "constructor", "prototype"]);

function unflatten(flat: Record<string, string>): MessageTree {
  const root = Object.create(null) as MessageTree;
  for (const [key, value] of Object.entries(flat)) {
    const parts = key.split(".");
    let node: MessageTree = root;
    let skipped = false;
    for (let i = 0; i < parts.length - 1; i++) {
      const part = parts[i];
      if (DANGEROUS_KEYS.has(part)) {
        skipped = true;
        break;
      }
      if (typeof node[part] !== "object" || node[part] === null) {
        node[part] = Object.create(null) as MessageTree;
      }
      node = node[part] as MessageTree;
    }
    const leaf = parts[parts.length - 1];
    if (!skipped && !DANGEROUS_KEYS.has(leaf)) {
      node[leaf] = value;
    }
  }
  return root;
}

async function resolveLocale(): Promise<string> {
  try {
    const cfg = await configApi.get();
    return cfg?.locale || DEFAULT_LOCALE;
  } catch {
    return DEFAULT_LOCALE;
  }
}

/** 应用启动时调用：解析语言 → 拉取消息表 → 创建 i18n 实例。 */
let instance: I18n | null = null;

/** 非组件环境（工具函数等）取全局 t；i18n 未就绪时返回 key 本身。 */
export function t(key: string): string {
  // legacy:false 下 global 是 Composer；此处只需要最朴素的 key→string 翻译。
  const g = instance?.global as unknown as { t: (k: string) => string } | undefined;
  return g?.t(key) ?? key;
}

export async function setupI18n(): Promise<I18n> {
  const locale = await resolveLocale();
  currentLocale = locale;
  const flat = await utilApi.i18nMessages(locale).catch(() => ({}));
  const i18n = createI18n({
    legacy: false,
    globalInjection: true,
    locale,
    fallbackLocale: DEFAULT_LOCALE,
    messages: { [locale]: unflatten(flat) },
  });
  instance = i18n;
  return i18n;
}

/** 切换语言：拉取新语言消息表并热替换。 */
export async function changeLocale(locale: string): Promise<void> {
  if (!instance) return;
  currentLocale = locale;
  const flat = await utilApi.i18nMessages(locale).catch(() => ({}));
  instance.global.setLocaleMessage(locale, unflatten(flat));
  // legacy:false 模式下 global 为 Composer，locale 是 WritableComputedRef<string>
  (instance.global as { locale: { value: string } }).locale.value = locale;
}
