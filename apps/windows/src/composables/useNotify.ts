// src/composables/useNotify.ts - 统一的成功 / 失败提示。
// 视图里不再各自拼 `ElMessage.error(String(e))` / `friendlyError`，文案策略集中在此处。

import { ElMessage } from "element-plus";
import { useI18n } from "vue-i18n";
import { friendlyError } from "@/errors";

export function useNotify() {
  const { t } = useI18n();
  return {
    /** 成功提示；缺省文案为 common.success。 */
    ok(message?: string) {
      ElMessage.success(message ?? t("common.success"));
    },
    /** 失败提示：识别得出的底层错误翻成人话，其余原样透出。 */
    fail(e: unknown, prefix?: string) {
      const text = friendlyError(e);
      ElMessage.error(prefix ? `${prefix}${text}` : text);
    },
    /** 已组装好的失败文案（不再套 friendlyError）。 */
    error(message: string) {
      ElMessage.error(message);
    },
    info(message: string) {
      ElMessage.info(message);
    },
  };
}
