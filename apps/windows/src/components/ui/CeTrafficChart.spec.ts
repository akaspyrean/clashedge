import { beforeEach, describe, expect, it, vi } from "vitest";
import { mountApp, stubBrowserApis } from "@/test/harness";
import CeTrafficChart from "./CeTrafficChart.vue";

const series = (n: number, v: number) => Array.from({ length: n }, (_, i) => v + i);

beforeEach(() => stubBrowserApis(vi));

describe("CeTrafficChart", () => {
  it("draws a down line with area fill and an up line, summarised for screen readers", () => {
    const w = mountApp(CeTrafficChart as never, { props: { down: series(60, 1000), up: series(60, 100) } });
    expect(w.find(".ce-chart__down").exists()).toBe(true);
    expect(w.find(".ce-chart__up").exists()).toBe(true);
    expect(w.find(".ce-chart__area").exists()).toBe(true);
    expect(w.find("svg").attributes("role")).toBe("img");
    expect(w.find("svg").attributes("aria-label")).toBe("ui.chart.aria");
  });

  it("has a single Y axis with four clean gridlines", () => {
    const w = mountApp(CeTrafficChart as never, { props: { down: series(60, 1000), up: series(60, 100) } });
    expect(w.findAll(".ce-chart__grid")).toHaveLength(4);
  });

  it("shows a crosshair and tooltip on hover, hides them on leave", async () => {
    const w = mountApp(CeTrafficChart as never, { props: { down: series(60, 1000), up: series(60, 100) } });
    expect(w.find(".ce-chart__cross").exists()).toBe(false);
    await w.find("svg").trigger("mousemove", { clientX: 300 });
    expect(w.find(".ce-chart__cross").exists()).toBe(true);
    expect(w.find(".ce-chart__tip").exists()).toBe(true);
    await w.find("svg").trigger("mouseleave");
    expect(w.find(".ce-chart__tip").exists()).toBe(false);
  });

  it("offers the same data as a table, newest first", async () => {
    const w = mountApp(CeTrafficChart as never, { props: { down: [10, 20, 30], up: [1, 2, 3] } });
    expect(w.find("table").exists()).toBe(false);
    const toggle = w.find(".ce-chart__toggle");
    expect(toggle.attributes("aria-expanded")).toBe("false");
    await toggle.trigger("click");
    expect(toggle.attributes("aria-expanded")).toBe("true");
    const rows = w.findAll("tbody tr");
    expect(rows).toHaveLength(3);
    expect(rows[0].text()).toContain("ui.chart.now");
    expect(rows[0].text()).toContain("30 B/s");
  });

  it("renders an empty window without throwing", () => {
    const w = mountApp(CeTrafficChart as never, { props: { down: [], up: [] } });
    expect(w.find(".ce-chart__down").exists()).toBe(false);
    expect(w.findAll(".ce-chart__grid")).toHaveLength(4);
  });
});
