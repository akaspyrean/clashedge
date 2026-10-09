import { describe, expect, it } from "vitest";
import { mountApp } from "@/test/harness";
import CeSparkline from "./CeSparkline.vue";

describe("CeSparkline", () => {
  it("draws both series and exposes a label", () => {
    const w = mountApp(CeSparkline as never, { props: { down: [1, 4, 2], up: [0, 1, 1], label: "近 60 秒" } });
    expect(w.attributes("role")).toBe("img");
    expect(w.attributes("aria-label")).toBe("近 60 秒");
    expect(w.find(".ce-spark__down").attributes("points")?.split(" ")).toHaveLength(3);
    expect(w.find(".ce-spark__up").exists()).toBe(true);
    expect(w.find(".ce-spark__area").exists()).toBe(true);
  });

  it("right-aligns a short series: the newest point sits at the right edge", () => {
    const w = mountApp(CeSparkline as never, { props: { down: [1, 2], up: [], label: "x" } });
    const pts = w.find(".ce-spark__down").attributes("points")!.split(" ");
    expect(pts[1].startsWith("300.0,")).toBe(true);
    expect(Number(pts[0].split(",")[0])).toBeGreaterThan(250);
  });

  it("renders nothing for empty data without throwing", () => {
    const w = mountApp(CeSparkline as never, { props: { down: [], up: [], label: "x" } });
    expect(w.find("polyline").exists()).toBe(false);
  });
});
