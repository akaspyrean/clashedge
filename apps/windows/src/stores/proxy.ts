// src/stores/proxy.ts
// 代理组状态：组列表、当前选择、延迟测试结果。
//
// 两类延迟分开存储：
// - delays[groupName]  —— 组延迟（GET /proxies/{group}/delay，测组当前选中节点）
// - nodeDelays[nodeName] —— 节点延迟（GET /proxies/{node}/delay，逐个测组内节点，
//   供「人工优选」手动测速选择节点使用）

import { defineStore } from "pinia";
import { proxyApi, type ProxyGroup } from "@/api/proxy";
import { isNodePoolDegraded } from "@/utils/latency";

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
    /** 重新测试单个节点（节点格上的延迟标签点击）；结果写入 nodeDelays。 */
    async testNode(name: string) {
      try {
        const r = (await proxyApi.testLatency(name))[0];
        this.nodeDelays[name] = r ? r.delay : null;
      } catch {
        this.nodeDelays[name] = null;
      }
    },
    /** 手动测速：对组内所有节点（排除 DIRECT 兜底）逐一测试延迟，
     *  结果写入 nodeDelays，供节点列表展示，辅助人工挑选。
     *  分块限并发（每批 10 个）防止节点过多时一次性全量并发压垮后端；
     *  不取消旧批次，仅通过 testingNodes 标志防重入。 */
    async testGroupProxies(group: string) {
      const g = this.groups.find((x) => x.name === group);
      if (!g) return;
      await this.testNodes(g.all.filter((p) => p !== "DIRECT"));
    },
    /** 批量测速指定节点（去重）：分块限并发（每批 10 个）防止一次性全量并发压垮后端；
     *  testingNodes 标志防重入。
     *
     *  自愈：若**所有**被测节点都不可用（全部超时，或全部延迟高于「较差」阈值，
     *  或两者混合），立即通知后端同时刷新「节点数据」与「地理数据/规则」
     *  （节点池整体异常通常是订阅过期或 GeoData/规则陈旧所致）。后端自带
     *  10 分钟冷却限流，重复触发不会打爆上游。 */
    async testNodes(names: string[]) {
      if (this.testingNodes) return;
      this.testingNodes = true;
      try {
        const nodes = [...new Set(names)];
        const CHUNK = 10;
        const testedDelays: Array<number | null | undefined> = [];
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
          for (const n of batch) testedDelays.push(this.nodeDelays[n]);
        }
        // 节点池整体不可用（全超时 / 全异常慢）→ 立即触发两类数据自愈更新
        if (isNodePoolDegraded(testedDelays)) {
          try {
            await proxyApi.triggerAutoRecovery();
          } catch {
            // 自愈失败不影响测速结果
          }
        }
      } finally {
        this.testingNodes = false;
      }
    },
  },
});
