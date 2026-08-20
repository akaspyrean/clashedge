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
  connections: ConnectionInfo[];
}

export const connectionsApi = {
  list: () => invoke<ConnectionsSummary>("get_connections"),
  closeAll: () => invoke<void>("close_all_connections"),
};
