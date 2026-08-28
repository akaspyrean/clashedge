// src/api/core.ts
// 核心（mihomo）命令的类型化封装

import { invoke } from "@tauri-apps/api/core";

export interface CoreStatus {
  running: boolean;
  status: string;
  version?: string | null;
}

export const coreApi = {
  getStatus: () => invoke<CoreStatus>("get_status"),
  start: () => invoke<void>("start_core"),
  stop: () => invoke<void>("stop_core"),
  restart: () => invoke<void>("restart_core"),
  reloadConfig: () => invoke<void>("reload_config"),
};
