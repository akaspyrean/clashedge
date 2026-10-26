import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const testLatency = vi.fn();
const triggerAutoRecovery = vi.fn();

vi.mock("@/api/proxy", () => ({
  proxyApi: {
    testLatency: (...args: unknown[]) => testLatency(...args),
    triggerAutoRecovery: (...args: unknown[]) => triggerAutoRecovery(...args),
    getGroups: vi.fn(async () => []),
    select: vi.fn(async () => undefined),
  },
}));

import { useProxyStore } from "./proxy";

/** 让每个节点返回指定延迟（0 = 超时）。 */
function mockDelays(delays: number[]) {
  testLatency.mockImplementation(async (name: string) => {
    const i = Number(name.replace(/\D/g, "")) || 0;
    return [{ group: name, delay: delays[i] ?? 0 }];
  });
}

describe("proxy store auto-recovery", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    testLatency.mockReset();
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

  it("triggers when every healthy-looking node is slower than the bad threshold", async () => {
    mockDelays([610, 929, 1200]);
    await useProxyStore().testNodes(["node0", "node1", "node2"]);
    expect(triggerAutoRecovery).toHaveBeenCalledTimes(1);
  });

  it("triggers when timeouts and abnormally slow nodes are mixed", async () => {
    mockDelays([0, 800, 0]);
    await useProxyStore().testNodes(["node0", "node1", "node2"]);
    expect(triggerAutoRecovery).toHaveBeenCalledTimes(1);
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

  it("keeps latency results even when the recovery call fails", async () => {
    triggerAutoRecovery.mockRejectedValue(new Error("offline"));
    mockDelays([0, 0]);
    const store = useProxyStore();
    await expect(store.testNodes(["node0", "node1"])).resolves.toBeUndefined();
    expect(store.nodeDelays.node0).toBe(0);
    expect(store.nodeDelays.node1).toBe(0);
    expect(store.testingNodes).toBe(false);
  });
});
