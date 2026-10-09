import { describe, expect, it } from "vitest";
import { classifyLatency } from "./latency";

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
