// src/composables/useSettingsSave.ts - 设置页「保存」：只提交与基线有差异的顶层键。
// 降级模式（config.yaml 损坏）下不提交：先在应用顶部横幅确认覆盖（后端同样拒绝）。

import { useI18n } from "vue-i18n";
import { useConfigStore } from "@/stores/config";
import { useAction } from "./useAction";
import { useNotify } from "./useNotify";

export function useSettingsSave() {
  const { t } = useI18n();
  const config = useConfigStore();
  const notify = useNotify();
  const { busy, run } = useAction();

  async function save() {
    if (config.degraded) {
      notify.fail(t("settings.degraded_save_blocked"));
      return;
    }
    await run(() => config.save(), { success: true });
  }

  return { saving: busy, save };
}
