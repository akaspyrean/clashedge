// src/stores/update.ts - 应用更新：签名清单检查 → 下载校验暂存 → 启动器安装。

import { defineStore } from "pinia";
import { updateApi, type PendingUpdate, type UpdateStatus } from "@/api/update";

export const useUpdateStore = defineStore("update", {
  state: () => ({
    status: null as UpdateStatus | null,
    staged: null as PendingUpdate | null,
    checking: false,
    downloading: false,
  }),
  getters: {
    availableVersion: (s) => (s.status?.status === "available" ? s.status.version : ""),
    upToDate: (s) => s.status?.status === "up_to_date",
  },
  actions: {
    async loadStaged() {
      this.staged = await updateApi.staged().catch(() => null);
    },
    /** 检查更新；失败抛出，由调用方决定是否提示（静默检查可忽略）。 */
    async check() {
      this.checking = true;
      try {
        this.status = await updateApi.check();
      } finally {
        this.checking = false;
      }
      return this.status;
    },
    async download() {
      this.downloading = true;
      try {
        this.staged = await updateApi.download();
      } finally {
        this.downloading = false;
      }
      return this.staged;
    },
    async discard() {
      await updateApi.discard();
      this.staged = null;
    },
    /** 重启并由启动器安装（成功时本进程即将退出）。 */
    restartAndApply: () => updateApi.restartAndApply(),
  },
});
