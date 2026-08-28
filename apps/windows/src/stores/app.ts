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
    async loadAutostart() {
      try {
        this.autostart = await utilApi.getAutostart();
      } catch {
        this.autostart = false;
      }
    },
  },
});
