// src/stores/app.ts
// 应用级状态：版本信息、支持的语言列表、开机自启。

import { defineStore } from "pinia";
import { utilApi } from "@/api/util";

export const useAppStore = defineStore("app", {
  state: () => ({
    version: "",
    locales: [] as string[],
    /** 开机自启的真实状态（注册表 Run 键）。由生命周期级 autostart-changed
     *  监听 + 启动时拉取维护，供设置页开关与托盘跨入口共享，避免页面级不同步。 */
    autostart: false,
  }),
  actions: {
    async init() {
      try {
        this.version = await utilApi.appVersion();
      } catch {
        this.version = "";
      }
      try {
        this.locales = await utilApi.locales();
      } catch {
        this.locales = ["zh-CN", "en-US"];
      }
    },
    /** 开机自启：后端写注册表成功后才更新状态（失败时开关保持原值并抛出）。 */
    async setAutostart(enable: boolean) {
      await utilApi.setAutostart(enable);
      this.autostart = enable;
    },
    openDataDir: () => utilApi.openDataDir(),
    /** 导出脱敏诊断包，返回文件路径。 */
    exportDiagnostics: () => utilApi.exportDiagnostics(),
    async loadAutostart() {
      try {
        this.autostart = await utilApi.getAutostart();
      } catch {
        this.autostart = false;
      }
    },
  },
});
