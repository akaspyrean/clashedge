// src/api/util.ts
// 工具命令的类型化封装（目录 / 版本 / 语言）

import { invoke } from "@tauri-apps/api/core";

export const utilApi = {
  openDataDir: () => invoke<void>("open_data_dir"),
  openLogsDir: () => invoke<void>("open_logs_dir"),
  appVersion: () => invoke<string>("get_app_version"),
  isAutostart: () => invoke<boolean>("is_autostart"),
  getAutostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (enable: boolean) => invoke<void>("set_autostart", { enable }),
  locales: () => invoke<string[]>("get_supported_locales"),
  setLocale: (locale: string) => invoke<void>("set_locale", { locale }),
  i18nMessages: (locale: string) =>
    invoke<Record<string, string>>("get_i18n_messages", { locale }),
};
