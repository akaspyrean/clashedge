// src/composables/useConfirm.ts - 二次确认对话框（Promise<boolean>）。

import { ElMessageBox } from "element-plus";
import { useI18n } from "vue-i18n";

export function useConfirm() {
  const { t } = useI18n();
  /** 用户确认返回 true，取消 / 关闭返回 false；永不抛出。 */
  return async function confirm(
    message: string,
    options: {
      title?: string;
      type?: "warning" | "info" | "error";
      confirmText?: string;
      cancelText?: string;
      /** 危险操作：确认按钮用危险红，且默认焦点放在「取消」上，防止回车误删。 */
      danger?: boolean;
    } = {},
  ): Promise<boolean> {
    try {
      if (options.danger) {
        // ElMessageBox 默认聚焦确认按钮；弹出后把焦点移到取消（Windows 习惯：取消在左，确认在右）。
        window.setTimeout(() => {
          document.querySelector<HTMLElement>(".ce-confirm-danger .el-message-box__btns .el-button")?.focus();
        }, 0);
      }
      await ElMessageBox.confirm(message, options.title ?? t("common.confirm"), {
        type: options.type ?? "warning",
        confirmButtonText: options.confirmText,
        cancelButtonText: options.cancelText,
        ...(options.danger ? { customClass: "ce-confirm-danger", confirmButtonClass: "el-button--danger" } : {}),
      });
      return true;
    } catch {
      return false;
    }
  };
}
