// src/stores/config.ts
// 配置中心：加载 / 保存 ClashConfig，提供 kebab-case 键的 getter。
// 注意：后端 Config 使用 `#[serde(rename_all = "kebab-case")]` + general 展开，
// 因此 JSON 键是 kebab-case（如 "mixed-port"、"allow-lan"）。

import { defineStore } from "pinia";
import { configApi, type ClashConfig } from "@/api/config";

export const useConfigStore = defineStore("config", {
  state: () => ({
    config: null as ClashConfig | null,
    /** load 成功时的基线快照：save() 只提交与它的顶层键差异，避免整包回传
     *  覆盖用户停留设置页期间托盘等其他入口改过的字段。 */
    baseline: null as ClashConfig | null,
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
      this.baseline = structuredClone(this.config);
      return this.config;
    },
    /** 计算当前配置相对基线的顶层键差异。
     *  顶层键含 tun/dns 等嵌套对象：structuredClone 出来的 baseline 与 config 是
     *  两个独立对象引用，`!==` 恒为 true，会让"只改一个普通字段保存"也把 tun/dns
     *  一并提交，重新引入旧嵌套对象覆盖托盘刚改值的回归。改为内容级比较
     *  （JSON 序列化对 Config 这种小数据足够且语义明确）。顶层键确实变了才整体
     *  提交该键（后端按键浅合并替换），与原契约一致。 */
    diffFromBaseline(): Partial<ClashConfig> {
      const patch: Partial<ClashConfig> = {};
      if (!this.config || !this.baseline) return patch;
      for (const key of Object.keys(this.config) as (keyof ClashConfig)[]) {
        const cur = this.config[key];
        const base = this.baseline[key];
        const same =
          JSON.stringify(cur) === JSON.stringify(base);
        if (!same) {
          (patch as Record<string, unknown>)[key] = cur;
        }
      }
      return patch;
    },
    async save() {
      // 失败时由调用方捕获处理；此处不修改内存状态。
      if (!this.config) return;
      const patch = this.diffFromBaseline();
      if (Object.keys(patch).length === 0) return;
      await configApi.updateFields(patch);
      // 成功后重新 load 刷新内存与基线（后端可能还有校验修正）。
      await this.load();
    },
    /** 就地修改部分字段后仅提交这些顶层键（update_config_fields 浅合并）。
     *  先调后端，成功后再改内存与基线，失败时抛错且内存不变（避免假保存）。 */
    async patch(partial: Partial<ClashConfig>) {
      if (!this.config) return;
      await configApi.updateFields(partial);
      const next = { ...this.config, ...partial };
      this.config = next;
      if (this.baseline) {
        const base: Record<string, unknown> = this.baseline;
        for (const key of Object.keys(partial)) {
          base[key] = structuredClone(next[key as keyof ClashConfig]);
        }
      }
    },
    async reset() {
      // 失败时抛错，由调用方处理并恢复内存状态。
      await configApi.reset();
      await this.load();
    },
  },
});
