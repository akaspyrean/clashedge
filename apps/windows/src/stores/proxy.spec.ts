import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const testNodeHealth = vi.fn();
const triggerAutoRecovery = vi.fn();

vi.mock("@/api/proxy", () => ({
  proxyApi: {
    testNodeHealth: (...args: unknown[]) => testNodeHealth(...args),
    triggerAutoRecovery: (...args: unknown[]) => triggerAutoRecovery(...args),
    getGroups: vi.fn(async () => []),
    getNodeHealth: vi.fn(async () => []),
    select: vi.fn(async () => undefined),
  },
}));

import { useProxyStore } from "./proxy";
import type { NodeHealthSummary } from "@/api/proxy";

/** 构造健康摘要：延迟 ≤ 0 视为探测失败（不可用）。 */
function healthSummary(node: string, delay: number): NodeHealthSummary {
  const available = delay > 0;
  return {
    node,
    score: available ? 90 : 0,
    availability: available ? 1 : 0,
    median_latency_ms: available ? delay : null,
    sessions: 1,
    last_available: available,
    last_latency_ms: available ? delay : null,
    last_tested_at_ms: 0,
  };
}

/** 让每个节点返回指定延迟（0 = 超时）。 */
function mockDelays(delays: number[]) {
  testNodeHealth.mockImplementation(async (name: string) => {
    const i = Number(name.replace(/\D/g, "")) || 0;
    return healthSummary(name, delays[i] ?? 0);
  });
}

describe("proxy store auto-recovery", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    testNodeHealth.mockReset();
    triggerAutoRecovery.mockReset();
    triggerAutoRecovery.mockResolvedValue({
      triggered: true,
      refreshed: 1,
      geo_updated: true,
      reason: "all nodes unavailable (timeout or abnormally slow)",
    });
  });

  it("triggers node + geo/rules refresh when every node timed out", async () => {
    mockDelays([0, 0, 0]);
    await useProxyStore().testNodes(["node0", "node1", "node2"]);
    expect(triggerAutoRecovery).toHaveBeenCalledTimes(1);
  });

  it("does not treat high-latency responses as complete outage", async () => {
    mockDelays([610, 929, 1200]);
    await useProxyStore().testNodes(["node0", "node1", "node2"]);
    expect(triggerAutoRecovery).not.toHaveBeenCalled();
  });

  it("does not trigger when at least one node responds despite high latency", async () => {
    mockDelays([0, 800, 0]);
    await useProxyStore().testNodes(["node0", "node1", "node2"]);
    expect(triggerAutoRecovery).not.toHaveBeenCalled();
  });

  it("does not trigger while at least one node is healthy", async () => {
    mockDelays([0, 900, 86]);
    await useProxyStore().testNodes(["node0", "node1", "node2"]);
    expect(triggerAutoRecovery).not.toHaveBeenCalled();
  });

  it("does not trigger for an empty node list", async () => {
    await useProxyStore().testNodes([]);
    expect(triggerAutoRecovery).not.toHaveBeenCalled();
  });

  it("ignores results from a previous profile after invalidation", async () => {
    let finish!: (value: NodeHealthSummary) => void;
    testNodeHealth.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    const store = useProxyStore();
    const pending = store.testNode("node1");
    store.invalidateLatencies();
    finish(healthSummary("node1", 30));
    await pending;
    expect(store.nodeDelays.node1).toBeUndefined();
  });

  it("keeps latency results even when the recovery call fails", async () => {
    triggerAutoRecovery.mockRejectedValue(new Error("offline"));
    mockDelays([0, 0]);
    const store = useProxyStore();
    await expect(store.testNodes(["node0", "node1"])).resolves.toBeUndefined();
    // 探测失败记为 null（失败），不是有效延迟
    expect(store.nodeDelays.node0).toBeNull();
    expect(store.nodeDelays.node1).toBeNull();
    expect(store.testingNodes).toBe(false);
  });
});
