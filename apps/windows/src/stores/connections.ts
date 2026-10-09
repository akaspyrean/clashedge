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

/** 速率曲线窗口（秒）。 */
export const RATE_WINDOW = 60;
/** 两次采样间隔超过该值视为中断（窗口隐藏 / 核心停止），清空曲线而不是画一条平线。 */
const RATE_GAP_RESET_SECONDS = 20;

export const useConnectionsStore = defineStore("connections", {
  state: () => ({
    connections: [] as ConnectionInfo[],
    /** 真实总数（后端可能已裁剪列表） */
    count: 0,
    truncated: false,
    downloadTotal: 0,
    uploadTotal: 0,
    /** 最近 RATE_WINDOW 秒的速率（字节/秒），每秒一格、最新在后。
     *  由相邻两次轮询的累计值求差得出，不新增后端接口、不另起轮询。 */
    rateDown: [] as number[],
    rateUp: [] as number[],
    lastSample: null as { at: number; down: number; up: number } | null,
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
      this.recordRate(this.downloadTotal, this.uploadTotal, Date.now());
    },
    /** 记录一次累计值：与上一次求差得到速率，并按经过的秒数补齐到每秒一格。
     *  计数回退（核心重启）按 0 处理；间隔过长（页面隐藏 / 核心停过）清空窗口重新开始。 */
    recordRate(down: number, up: number, at: number) {
      const last = this.lastSample;
      this.lastSample = { at, down, up };
      if (!last || at <= last.at) return;
      const dt = (at - last.at) / 1000;
      if (dt > RATE_GAP_RESET_SECONDS) {
        this.rateDown = [];
        this.rateUp = [];
        return;
      }
      const rd = down >= last.down ? (down - last.down) / dt : 0;
      const ru = up >= last.up ? (up - last.up) / dt : 0;
      const n = Math.min(RATE_WINDOW, Math.max(1, Math.round(dt)));
      this.rateDown = [...this.rateDown, ...Array<number>(n).fill(rd)].slice(-RATE_WINDOW);
      this.rateUp = [...this.rateUp, ...Array<number>(n).fill(ru)].slice(-RATE_WINDOW);
    },
    closeAll: () => connectionsApi.closeAll(),
  },
});
