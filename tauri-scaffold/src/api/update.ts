// src/api/update.ts
// Portable Updater API（0.8.10）

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
  download(manifest: UpdateManifest): Promise<PendingUpdate> {
    return invoke("download_update", {
      version: manifest.version,
      url: manifest.url,
      sha256: manifest.sha256,
    });
  },
  staged(): Promise<PendingUpdate | null> {
    return invoke("get_staged_update");
  },
  discard(): Promise<void> {
    return invoke("discard_staged_update");
  },
};
