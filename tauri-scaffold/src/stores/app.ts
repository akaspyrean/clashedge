// src/stores/app.ts
// 应用级状态：版本信息、支持的语言列表。

import { defineStore } from "pinia";
import { utilApi } from "@/api/util";

export const useAppStore = defineStore("app", {
  state: () => ({
    version: "",
    locales: [] as string[],
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
  },
});
