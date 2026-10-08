// src/composables/useConfirm.ts - 二次确认对话框（Promise<boolean>）。

import { ElMessageBox } from "element-plus";
import { useI18n } from "vue-i18n";

export function useConfirm() {
  const { t } = useI18n();
  /** 用户确认返回 true，取消 / 关闭返回 false；永不抛出。 */
  return async function confirm(
    message: string,
    options: { title?: string; type?: "warning" | "info" | "error"; confirmText?: string; cancelText?: string } = {},
  ): Promise<boolean> {
    try {
      await ElMessageBox.confirm(message, options.title ?? t("common.confirm"), {
        type: options.type ?? "warning",
        confirmButtonText: options.confirmText,
        cancelButtonText: options.cancelText,
      });
      return true;
    } catch {
      return false;
    }
  };
}
