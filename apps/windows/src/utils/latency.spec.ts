import { describe, expect, it } from "vitest";
import { classifyLatency, isNodePoolDegraded } from "./latency";

describe("classifyLatency", () => {
  it("grades by the 200 / 500 ms boundaries", () => {
    expect(classifyLatency({ ms: 86 })).toBe("good");
    expect(classifyLatency({ ms: 200 })).toBe("good");
    expect(classifyLatency({ ms: 201 })).toBe("mid");
    expect(classifyLatency({ ms: 500 })).toBe("mid");
    expect(classifyLatency({ ms: 501 })).toBe("bad");
  });

  it("treats missing data as untested and zero as failure", () => {
    expect(classifyLatency({})).toBe("none");
    expect(classifyLatency({ ms: null })).toBe("none");
    expect(classifyLatency({ ms: 0 })).toBe("timeout");
  });

  it("lets testing / timeout flags win over a stale value", () => {
    expect(classifyLatency({ ms: 86, testing: true })).toBe("testing");
    expect(classifyLatency({ ms: 86, timeout: true })).toBe("timeout");
  });

  it("falls back to defaults for thresholds passed as undefined", () => {
    expect(classifyLatency({ ms: 86, thresholds: { good: undefined, bad: undefined } })).toBe("good");
  });

  it("honours custom thresholds", () => {
    expect(classifyLatency({ ms: 150, thresholds: { good: 100, bad: 300 } })).toBe("mid");
  });
});

describe("isNodePoolDegraded", () => {
  it("is false for an empty pool or any healthy node", () => {
    expect(isNodePoolDegraded([])).toBe(false);
    expect(isNodePoolDegraded([null, 0, 900, 320])).toBe(false);
    expect(isNodePoolDegraded([86])).toBe(false);
    expect(isNodePoolDegraded([500])).toBe(false); // 恰好处于「一般」上限，仍算可用
  });

  it("is true when every node timed out", () => {
    expect(isNodePoolDegraded([null, null, 0, undefined])).toBe(true);
  });

  it("is true when every node is slower than the bad threshold", () => {
    expect(isNodePoolDegraded([501, 610, 929])).toBe(true);
  });

  it("is true when timeouts and abnormally slow nodes are mixed", () => {
    expect(isNodePoolDegraded([null, 0, 800, Infinity])).toBe(true);
  });

  it("honours a custom bad threshold", () => {
    expect(isNodePoolDegraded([400, 600], { bad: 300 })).toBe(true);
    expect(isNodePoolDegraded([400, 600], { bad: 700 })).toBe(false);
  });
});
