// src/stores/config.ts
// 配置中心：加载 / 保存 ClashConfig，提供 kebab-case 键的 getter。
// 注意：后端 Config 使用 `#[serde(rename_all = "kebab-case")]` + general 展开，
// 因此 JSON 键是 kebab-case（如 "mixed-port"、"allow-lan"）。

import { defineStore } from "pinia";
import { configApi, type ClashConfig } from "@/api/config";

export const useConfigStore = defineStore("config", {
  state: () => ({
    config: null as ClashConfig | null,
  }),
  getters: {
    mixedPort: (s) => s.config?.["mixed-port"] ?? 7890,
    allowLan: (s) => s.config?.["allow-lan"] ?? false,
    logLevel: (s) => s.config?.["log-level"] ?? "info",
    proxyMode: (s) => s.config?.mode ?? "rule",
    systemProxy: (s) => s.config?.["system-proxy"] ?? false,
    locale: (s) => s.config?.locale ?? "zh-CN",
    tunEnabled: (s) => s.config?.tun?.enable ?? false,
    mixinEnabled: (s) => s.config?.["mixin-enabled"] ?? false,
  },
  actions: {
    async load() {
      this.config = await configApi.get();
      return this.config;
    },
    async save() {
      // 失败时由调用方捕获处理；此处不修改内存状态。
      if (!this.config) return;
      await configApi.update(this.config);
    },
    /** 就地修改部分字段后整体保存。
     *  先调后端 update_config，成功后再改内存，失败时抛错且内存不变（避免假保存）。 */
    async patch(partial: Partial<ClashConfig>) {
      if (!this.config) return;
      const next = { ...this.config, ...partial };
      await configApi.update(next);
      this.config = next;
    },
    async reset() {
      // 失败时抛错，由调用方处理并恢复内存状态。
      await configApi.reset();
      await this.load();
    },
  },
});
