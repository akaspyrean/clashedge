import { describe, expect, it } from "vitest";
import { mountApp } from "@/test/harness";
import CeLatencyTag from "./CeLatencyTag.vue";

const mountTag = (props: Record<string, unknown>) => mountApp(CeLatencyTag as never, { props });

describe("CeLatencyTag", () => {
  it.each([
    [{ ms: 86 }, "ce-lat--good", "86 ms"],
    [{ ms: 320 }, "ce-lat--mid", "320 ms"],
    [{ ms: 860 }, "ce-lat--bad", "860 ms"],
    [{ ms: 0 }, "ce-lat--timeout", "ui.latency.timeout"],
    [{ ms: 86, timeout: true }, "ce-lat--timeout", "ui.latency.timeout"],
    [{ testing: true }, "ce-lat--testing", "ui.latency.testing"],
    [{}, "ce-lat--none", "– ms"],
  ])("maps %j to %s", (props, cls, text) => {
    const w = mountTag(props);
    expect(w.classes()).toContain(cls);
    expect(w.text()).toBe(text);
  });

  it("honours custom thresholds", () => {
    expect(mountTag({ ms: 150, good: 100, bad: 300 }).classes()).toContain("ce-lat--mid");
  });

  it("exposes a spoken description, not just colour", () => {
    const w = mountTag({ ms: 86 });
    expect(w.element.tagName).toBe("SPAN");
    expect(w.attributes("role")).toBe("img");
    expect(w.attributes("aria-label")).toBe("ui.latency.aria");
  });

  it("renders a button that emits retest when clickable", async () => {
    const w = mountTag({ ms: 86, clickable: true, name: "HK 01" });
    expect(w.element.tagName).toBe("BUTTON");
    expect(w.attributes("aria-label")).toContain("ui.latency.retest");
    await w.trigger("click");
    expect(w.emitted("retest")).toHaveLength(1);
  });

  it("marks a testing button as busy", () => {
    expect(mountTag({ testing: true, clickable: true }).attributes("aria-busy")).toBe("true");
  });
});
