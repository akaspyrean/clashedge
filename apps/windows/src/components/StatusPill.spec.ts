import { describe, expect, it } from "vitest";
import { mountApp } from "@/test/harness";
import StatusPill from "./StatusPill.vue";

describe("StatusPill", () => {
  it("always renders text plus a shaped dot (never colour alone)", () => {
    const w = mountApp(StatusPill as never, { props: { active: true, label: "运行中" } });
    expect(w.text()).toBe("运行中");
    expect(w.find(".status-dot").attributes("aria-hidden")).toBe("true");
    expect(w.attributes("role")).toBe("status");
  });

  it("infers ok / idle from active when no tone is given", () => {
    expect(mountApp(StatusPill as never, { props: { active: true, label: "a" } }).classes()).toContain("tone-ok");
    expect(mountApp(StatusPill as never, { props: { active: false, label: "a" } }).classes()).toContain("tone-idle");
  });

  it("lets an explicit tone win", () => {
    const w = mountApp(StatusPill as never, { props: { active: false, label: "a", tone: "error" } });
    expect(w.classes()).toContain("tone-error");
    expect(w.classes()).not.toContain("running");
  });
});
