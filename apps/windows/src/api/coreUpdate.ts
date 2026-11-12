// src/api/coreUpdate.ts
// 独立 Mihomo 核心更新 API（签名清单 → SHA-256 校验 → 版本化暂存）。

import { invoke } from "@tauri-apps/api/core";

export interface CoreUpdateStatus {
  status: "up_to_date" | "available";
  current: string;
  version?: string;
}

export interface StagedCoreUpdate {
  version: string;
  sha256: string;
  restart_required: boolean;
}

export const coreUpdateApi = {
  chainReady: () => invoke<boolean>("core_update_chain_ready"),
  check: () => invoke<CoreUpdateStatus>("check_core_update"),
  download: () => invoke<StagedCoreUpdate>("download_core_update"),
  staged: () => invoke<StagedCoreUpdate | null>("get_staged_core_update"),
};
