// src/stores/updateStatus.ts - 自动更新状态（更新时间 / 成功失败）。
//
// 数据来源：后端 `get_auto_update_status`（读 Data/auto-update-status.json）。
// 该文件由后端在每次自动更新（GeoData/规则集、订阅静默刷新）后写入，
// 因此首页可以如实告诉用户"上次自动更新是什么时候、成功了还是失败了"。

import { defineStore } from "pinia";
import { updateStatusApi, type AutoUpdateStatus } from "@/api/updateStatus";

export const useUpdateStatusStore = defineStore("updateStatus", {
  state: () => ({
    status: null as AutoUpdateStatus | null,
    loading: false,
  }),
  actions: {
    async refresh() {
      this.loading = true;
      try {
        this.status = await updateStatusApi.status();
      } catch {
        this.status = null;
      } finally {
        this.loading = false;
      }
    },
  },
});
