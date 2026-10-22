// src/api/updateStatus.ts
// 自动更新状态（首页展示"何时更新、成功还是失败"）

import { invoke } from "@tauri-apps/api/core";

/** 单项自动更新记录 */
export interface AutoUpdateRecord {
  /** 最后一次尝试（unix 秒，0 = 从未尝试） */
  attempt: number;
  /** 最后一次成功（unix 秒，0 = 从未成功） */
  success: number;
  /** 最后一次尝试是否成功 */
  ok: boolean;
  /** 失败原因（成功时为空串） */
  error: string;
}

export interface AutoUpdateStatus {
  /** GeoIP/GeoSite + 规则集更新 */
  geo: AutoUpdateRecord;
  /** 各订阅的自动刷新结果（键为 profile 名） */
  subscriptions: Record<string, AutoUpdateRecord>;
  /** GeoData 自动更新开关 */
  geo_enabled: boolean;
  /** 订阅自动刷新开关 */
  subscription_enabled: boolean;
}

export const updateStatusApi = {
  status: () => invoke<AutoUpdateStatus>("get_auto_update_status"),
};
