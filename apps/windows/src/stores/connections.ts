// src/stores/connections.ts - 连接列表快照（轮询由视图的生命周期驱动）。

import { defineStore } from "pinia";
import { connectionsApi, type ConnectionInfo } from "@/api/connections";

/** 连接越多轮询越慢，避免大量连接时拖垮 IPC / WebView。 */
export function pollIntervalFor(count: number): number {
  if (count < 200) return 2000;
  if (count < 1000) return 3000;
  if (count < 5000) return 5000;
  return 8000;
}

export const useConnectionsStore = defineStore("connections", {
  state: () => ({
    connections: [] as ConnectionInfo[],
    /** 真实总数（后端可能已裁剪列表） */
    count: 0,
    truncated: false,
    downloadTotal: 0,
    uploadTotal: 0,
  }),
  actions: {
    /** 拉取一次快照；失败抛出（轮询方静默）。 */
    async refresh() {
      const data = await connectionsApi.list();
      const all = data.connections ?? [];
      this.count = data.total ?? all.length;
      this.truncated = data.truncated ?? false;
      this.connections = all;
      this.downloadTotal = data.download_total ?? 0;
      this.uploadTotal = data.upload_total ?? 0;
    },
    closeAll: () => connectionsApi.closeAll(),
  },
});
