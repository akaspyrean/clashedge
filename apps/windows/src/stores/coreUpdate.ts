// src/stores/coreUpdate.ts - Mihomo 核心更新独立于便携应用更新。

import { defineStore } from "pinia";
import { coreUpdateApi, type CoreUpdateStatus, type StagedCoreUpdate } from "@/api/coreUpdate";

export const useCoreUpdateStore = defineStore("coreUpdate", {
  state: () => ({
    status: null as CoreUpdateStatus | null,
    staged: null as StagedCoreUpdate | null,
    checking: false,
    downloading: false,
    chainReady: false,
  }),
  getters: {
    availableVersion: (state) =>
      state.status?.status === "available" ? state.status.version ?? "" : "",
    upToDate: (state) => state.status?.status === "up_to_date",
  },
  actions: {
    async loadChainReady() {
      this.chainReady = await coreUpdateApi.chainReady().catch(() => false);
    },
    async loadStaged() {
      this.staged = await coreUpdateApi.staged().catch(() => null);
    },
    async check() {
      this.checking = true;
      try {
        this.status = await coreUpdateApi.check();
      } finally {
        this.checking = false;
      }
      return this.status;
    },
    async download() {
      this.downloading = true;
      try {
        this.staged = await coreUpdateApi.download();
      } finally {
        this.downloading = false;
      }
      return this.staged;
    },
  },
});
