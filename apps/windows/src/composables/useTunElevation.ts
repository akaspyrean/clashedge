// src/composables/useTunElevation.ts - 非管理员开启 TUN 的 UAC 提权重启流程。
//
// set_tun_mode 在权限预检失败时返回含「管理员权限」的错误（core/runtime.rs
// validate_tun_permission）。识别到该错误后弹二次确认；用户同意则调用
// relaunch_for_tun：UAC 授权 → 旧实例退出 → 新实例以管理员启动、
// 内核就绪后自动开启 TUN（main.rs 的 --clash-edge-enable-tun 路径）。
// 作为 useAction 的 onError 接入：已处理的错误不再叠加默认错误 toast。

import { useI18n } from "vue-i18n";
import { proxyApi } from "@/api/proxy";
import { useConfirm } from "./useConfirm";
import { useNotify } from "./useNotify";

/** 是否为「开启 TUN 缺少管理员权限」错误（与后端 validate_tun_permission 文案耦合）。 */
export function isTunElevationError(e: unknown): boolean {
  return String(e).includes("管理员权限");
}

export function useTunElevation() {
  const { t } = useI18n();
  const confirm = useConfirm();
  const notify = useNotify();

  /** useAction 的 onError：已处理返回 true（不弹默认错误 toast）。 */
  async function handleError(e: unknown): Promise<boolean> {
    if (!isTunElevationError(e)) return false;
    const ok = await confirm(t("tun.elevation_body"), {
      title: t("tun.elevation_title"),
      confirmText: t("tun.elevation_confirm"),
      cancelText: t("common.cancel"),
    });
    if (!ok) return true; // 用户取消：静默收场，不叠加错误提示
    try {
      await proxyApi.relaunchForTun(); // UAC 后旧实例退出，此 promise 通常不会 resolve
    } catch (err) {
      notify.fail(err); // 如用户在 UAC 弹窗点了「否」
    }
    return true;
  }

  return { handleError };
}
