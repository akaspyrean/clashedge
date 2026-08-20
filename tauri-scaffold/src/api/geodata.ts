// src/api/geodata.ts
// 地理数据命令的类型化封装（GeoIP / GeoSite 更新与状态）

import { invoke } from "@tauri-apps/api/core";

export interface GeoDataFileStatus {
  exists: boolean;
  size: number;
  url: string;
}

export interface GeoDataStatus {
  geoip: GeoDataFileStatus;
  geosite: GeoDataFileStatus;
}

export interface GeoDataUrls {
  geox_url: string;
  geoip_url: string;
  geosite_url: string;
}

export const geodataApi = {
  update: () => invoke<void>("update_geodata"),
  status: () => invoke<GeoDataStatus>("get_geodata_status"),
  getUrls: () => invoke<GeoDataUrls>("get_geodata_urls"),
  setUrls: (urls: Partial<GeoDataUrls>) =>
    invoke<void>("set_geodata_urls", { urls }),
};
