import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { RATE_WINDOW, useConnectionsStore } from "./connections";

describe("connections store · rate history", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("derives bytes/second from the delta of two cumulative samples", () => {
    const s = useConnectionsStore();
    s.recordRate(1_000, 100, 0);
    expect(s.rateDown).toEqual([]); // 第一次只建立基线
    s.recordRate(3_000, 300, 1_000);
    expect(s.rateDown).toEqual([2_000]);
    expect(s.rateUp).toEqual([200]);
  });

  it("fills one slot per elapsed second so a 2s poll still yields a per-second series", () => {
    const s = useConnectionsStore();
    s.recordRate(0, 0, 0);
    s.recordRate(4_000, 0, 2_000);
    expect(s.rateDown).toEqual([2_000, 2_000]);
  });

  it("treats a counter reset (core restart) as zero, not a negative rate", () => {
    const s = useConnectionsStore();
    s.recordRate(9_000, 9_000, 0);
    s.recordRate(10, 10, 1_000);
    expect(s.rateDown).toEqual([0]);
    expect(s.rateUp).toEqual([0]);
  });

  it("clears the window after a long gap instead of drawing a flat line", () => {
    const s = useConnectionsStore();
    s.recordRate(0, 0, 0);
    s.recordRate(100, 0, 1_000);
    expect(s.rateDown).toHaveLength(1);
    s.recordRate(5_000, 0, 600_000);
    expect(s.rateDown).toEqual([]);
  });

  it("keeps at most RATE_WINDOW samples, newest last", () => {
    const s = useConnectionsStore();
    s.recordRate(0, 0, 0);
    for (let i = 1; i <= RATE_WINDOW + 10; i++) s.recordRate(i * 100, 0, i * 1_000);
    expect(s.rateDown).toHaveLength(RATE_WINDOW);
    expect(s.rateDown[RATE_WINDOW - 1]).toBe(100);
  });
});
