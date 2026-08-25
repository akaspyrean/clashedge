// src/stores/core.ts
// 核心（mihomo）运行状态：轮询 get_status，提供启动 / 停止 / 重启 / 重载。

import { defineStore } from "pinia";
import { coreApi, type CoreStatus } from "@/api/core";

const IDLE: CoreStatus = { running: false, status: "stopped", version: null };

export const useCoreStore = defineStore("core", {
  state: () => ({
    status: { ...IDLE } as CoreStatus,
    starting: false,
    stopping: false,
  }),
  actions: {
    async refresh() {
      try {
        this.status = await coreApi.getStatus();
      } catch {
        this.status = { ...IDLE };
      }
      return this.status;
    },
    async start() {
      this.starting = true;
      try {
        await coreApi.start();
      } finally {
        this.starting = false;
        // 无论成败都刷新真实状态，避免失败时界面滞留旧值。
        await this.refresh().catch(() => {});
      }
    },
    async stop() {
      this.stopping = true;
      try {
        await coreApi.stop();
      } finally {
        this.stopping = false;
        await this.refresh().catch(() => {});
      }
    },
    async restart() {
      try {
        await coreApi.restart();
      } finally {
        await this.refresh().catch(() => {});
      }
    },
    async reload() {
      try {
        await coreApi.reloadConfig();
      } finally {
        await this.refresh().catch(() => {});
      }
    },
  },
});
