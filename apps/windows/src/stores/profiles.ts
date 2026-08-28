// src/stores/profiles.ts
// 配置文件状态：列表、增删改、重命名、激活。

import { defineStore } from "pinia";
import { profilesApi, type ProfileInfo } from "@/api/profiles";

export const useProfilesStore = defineStore("profiles", {
  state: () => ({
    profiles: [] as ProfileInfo[],
    loading: false,
  }),
  getters: {
    activeProfile: (s) => s.profiles.find((p) => p.active) ?? null,
  },
  actions: {
    async list() {
      this.loading = true;
      try {
        this.profiles = await profilesApi.list();
      } catch {
        this.profiles = [];
      } finally {
        this.loading = false;
      }
    },
    async create(name: string, content?: string) {
      await profilesApi.create(name, content);
      await this.list();
    },
    async remove(name: string) {
      await profilesApi.remove(name);
      await this.list();
    },
    async rename(oldName: string, newName: string) {
      await profilesApi.rename(oldName, newName);
      await this.list();
    },
    async activate(name: string) {
      await profilesApi.activate(name);
      this.profiles = this.profiles.map((p) => ({
        ...p,
        active: p.name === name,
      }));
    },
  },
});
