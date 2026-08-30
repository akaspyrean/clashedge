// src/api/update.ts
// Portable Updater API（签名信任链）
//
// 下载不再接受任何前端参数——后端只使用 check_update 刚验签过的
// manifest（minisign 签名验证通过后缓存于后端）。

import { invoke } from "@tauri-apps/api/core";

export interface UpdateManifest {
  version: string;
  url: string;
  sha256: string;
  notes?: string;
}

export type UpdateStatus =
  | { status: "up_to_date"; current: string }
  | ({ status: "available"; current: string } & UpdateManifest);

export interface PendingUpdate {
  version: string;
  zip_path: string;
  sha256: string;
}

export const updateApi = {
  check(): Promise<UpdateStatus> {
    return invoke("check_update");
  },
  download(): Promise<PendingUpdate> {
    return invoke("download_update");
  },
  staged(): Promise<PendingUpdate | null> {
    return invoke("get_staged_update");
  },
  discard(): Promise<void> {
    return invoke("discard_staged_update");
  },
};
