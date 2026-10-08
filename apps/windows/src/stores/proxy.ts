// src/stores/proxy.ts
// 代理组状态：组列表、当前选择、延迟测试结果。
//
// 两类延迟分开存储：
// - delays[groupName]  —— 组延迟（GET /proxies/{group}/delay，测组当前选中节点）
// - nodeDelays[nodeName] —— 节点延迟（GET /proxies/{node}/delay，逐个测组内节点，
//   供「人工优选」手动测速选择节点使用）

import { defineStore } from "pinia";
import { proxyApi, type ProxyGroup } from "@/api/proxy";

export const useProxyStore = defineStore("proxy", {
  state: () => ({
    groups: [] as ProxyGroup[],
    /** groupName -> delay(ms)，失败为 null */
    delays: {} as Record<string, number | null>,
    /** nodeName -> delay(ms)；undefined=未测，null=测过失败 */
    nodeDelays: {} as Record<string, number | undefined | null>,
    testing: false,
    /** 组内节点逐个测速中（人工优选手动测速） */
    testingNodes: false,
  }),
  actions: {
    async loadGroups() {
      try {
        // 载入全部代理组；可见性按当前模式由 ProxiesView 联动决定
        // （rule 隐藏 GLOBAL / global 独占显示 / direct 无组）。
        this.groups = await proxyApi.getGroups();
      } catch {
        this.groups = [];
      }
    },
    async select(group: string, proxy: string) {
      await proxyApi.select(group, proxy);
      await this.loadGroups();
    },
    /** 测试单个组（其当前选中节点的延迟）。 */
    async testOne(group: string) {
      try {
        const results = await proxyApi.testLatency(group);
        const r = results[0];
        this.delays[group] = r ? r.delay : null;
      } catch {
        this.delays[group] = null;
      }
    },
    /** 并发测试指定组；缺省为当前全部组。testing 置位后重入直接忽略（按钮 loading 已禁用）。 */
    async testAll(names?: string[]) {
      if (this.testing) return;
      this.testing = true;
      try {
        const targets = names ?? this.groups.map((g) => g.name);
        await Promise.all(targets.map((name) => this.testOne(name)));
      } finally {
        this.testing = false;
      }
    },
    /** 手动测速：对组内所有节点（排除 DIRECT 兜底）逐一测试延迟，
     *  结果写入 nodeDelays，供节点列表展示，辅助人工挑选。
     *  分块限并发（每批 10 个）防止节点过多时一次性全量并发压垮后端；
     *  不取消旧批次，仅通过 testingNodes 标志防重入。 */
    async testGroupProxies(group: string) {
      const g = this.groups.find((x) => x.name === group);
      if (!g) return;
      const nodes = g.all.filter((p) => p !== "DIRECT");
      if (this.testingNodes) return;
      this.testingNodes = true;
      try {
        const CHUNK = 10;
        for (let i = 0; i < nodes.length; i += CHUNK) {
          const batch = nodes.slice(i, i + CHUNK);
          await Promise.allSettled(
            batch.map(async (n) => {
              try {
                const r = (await proxyApi.testLatency(n))[0];
                this.nodeDelays[n] = r ? r.delay : null;
              } catch {
                this.nodeDelays[n] = null;
              }
            })
          );
        }
      } finally {
        this.testingNodes = false;
      }
    },
  },
});
