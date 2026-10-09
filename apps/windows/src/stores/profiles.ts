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
        this.profiles = (await profilesApi.list()) ?? [];
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
    async importFromUrl(name: string, url: string, userAgent?: string) {
      await profilesApi.importFromUrl(name, url, userAgent);
      await this.list();
    },
    async importContent(name: string, content: string) {
      await profilesApi.import(name, content);
      await this.list();
    },
    /** 导出：返回完整 YAML 文本，不改变列表。 */
    exportContent: (name: string) => profilesApi.export(name),
    /** 重新拉取订阅内容（激活中则后端热重载生效）。 */
    async refreshSubscription(name: string) {
      await profilesApi.updateProfile(name);
      await this.list();
    },
    async setUserAgent(name: string, userAgent: string | null) {
      await profilesApi.setUserAgent(name, userAgent);
      await this.list();
    },
    getContent: (name: string) => profilesApi.getContent(name),
    updateContent: (name: string, content: string) => profilesApi.updateContent(name, content),
    async activate(name: string) {
      await profilesApi.activate(name);
      this.profiles = this.profiles.map((p) => ({
        ...p,
        active: p.name === name,
      }));
    },
  },
});
