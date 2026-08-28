// src/api/connections.ts
// 连接列表命令的类型化封装（get_connections / close_all_connections）。

import { invoke } from "@tauri-apps/api/core";

export interface ConnectionInfo {
  id: string;
  host: string;
  network: string;
  type: string;
  rule: string;
  upload: number;
  download: number;
  /** Unix 毫秒时间戳，连接建立时刻。 */
  start: number;
  chains: string[];
}

export interface ConnectionsSummary {
  download_total: number;
  upload_total: number;
  /** 连接总数（真实值，可能大于 connections.length——后端已裁剪） */
  total: number;
  /** 是否因连接数超过后端上限而被裁剪（前端据此显示截断提示） */
  truncated: boolean;
  /** 后端裁剪后的连接列表（≤ 后端 MAX_CONNECTIONS_RETURNED） */
  connections: ConnectionInfo[];
}

export const connectionsApi = {
  list: () => invoke<ConnectionsSummary>("get_connections"),
  closeAll: () => invoke<void>("close_all_connections"),
};
