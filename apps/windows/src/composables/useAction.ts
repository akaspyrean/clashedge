// src/composables/useAction.ts - 带 in-flight 守卫的异步动作。
//
// 视图里"置 busy → try → 成功提示 → catch 失败提示 → finally 复位"的样板代码集中到这里：
// 同一动作在途时重复触发直接忽略（防连点重复提交），失败统一走 useNotify。

import { ref, type Ref } from "vue";
import { useNotify } from "./useNotify";

export interface ActionOptions {
  /** 成功后是否提示；true = 默认文案，字符串 = 自定义文案。缺省不提示。 */
  success?: boolean | string;
  /** 失败提示前缀。 */
  errorPrefix?: string;
  /** 失败时静默（不弹提示），仍返回 undefined。 */
  silent?: boolean;
}

export function useAction(shared?: Ref<boolean>) {
  const busy = shared ?? ref(false);
  const notify = useNotify();

  /** 执行动作；`ok=false` 表示失败或被在途守卫忽略（void 动作也能区分成败）。 */
  async function run<T>(
    fn: () => Promise<T>,
    opts: ActionOptions = {},
  ): Promise<{ ok: true; value: T } | { ok: false }> {
    if (busy.value) return { ok: false };
    busy.value = true;
    try {
      const value = await fn();
      if (opts.success) notify.ok(typeof opts.success === "string" ? opts.success : undefined);
      return { ok: true, value };
    } catch (e) {
      if (!opts.silent) notify.fail(e, opts.errorPrefix);
      return { ok: false };
    } finally {
      busy.value = false;
    }
  }

  return { busy, run };
}
