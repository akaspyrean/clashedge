// src/stores/geodata.ts - GeoIP / GeoSite 文件状态与更新。

import { defineStore } from "pinia";
import { geodataApi, type GeoDataStatus } from "@/api/geodata";

export const useGeodataStore = defineStore("geodata", {
  state: () => ({
    status: null as GeoDataStatus | null,
    updating: false,
  }),
  actions: {
    async refresh() {
      try {
        this.status = await geodataApi.status();
      } catch {
        this.status = null;
      }
    },
    async update() {
      this.updating = true;
      try {
        await geodataApi.update();
        await this.refresh();
      } finally {
        this.updating = false;
      }
    },
  },
});
