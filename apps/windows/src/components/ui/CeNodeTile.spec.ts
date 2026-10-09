import { describe, expect, it } from "vitest";
import { mountApp } from "@/test/harness";
import CeNodeTile from "./CeNodeTile.vue";

const mountTile = (props: Record<string, unknown> = {}) =>
  mountApp(CeNodeTile as never, { props: { name: "HK 01", protocol: "VMess", ms: 86, ...props } });

describe("CeNodeTile", () => {
  it("shows name, protocol and a latency tag", () => {
    const w = mountTile();
    expect(w.text()).toContain("HK 01");
    expect(w.text()).toContain("VMess");
    expect(w.find(".ce-lat--good").exists()).toBe(true);
  });

  it("never nests a button inside a button", () => {
    const w = mountTile();
    expect(w.findAll("button button")).toHaveLength(0);
    expect(w.findAll("button")).toHaveLength(2);
  });

  it("emits select from the main button and retest from the latency tag without bubbling", async () => {
    const w = mountTile();
    await w.find(".ce-lat").trigger("click");
    expect(w.emitted("retest")).toHaveLength(1);
    expect(w.emitted("select")).toBeUndefined();
    await w.find(".ce-tile__main").trigger("click");
    expect(w.emitted("select")).toHaveLength(1);
  });

  it("reflects the selected state with aria-pressed, a check mark and a class", () => {
    const w = mountTile({ selected: true });
    expect(w.classes()).toContain("is-selected");
    expect(w.find(".ce-tile__main").attributes("aria-pressed")).toBe("true");
    expect(w.find(".ce-tile__check").exists()).toBe(true);
    expect(mountTile().find(".ce-tile__check").exists()).toBe(false);
  });

  it("does not emit select when readonly, and is marked aria-disabled", async () => {
    const w = mountTile({ readonly: true });
    await w.find(".ce-tile__main").trigger("click");
    expect(w.emitted("select")).toBeUndefined();
    expect(w.find(".ce-tile__main").attributes("aria-disabled")).toBe("true");
    expect(w.classes()).toContain("is-readonly");
  });

  it("dims an unavailable node", () => {
    expect(mountTile({ unavailable: true, timeout: true }).classes()).toContain("is-unavailable");
  });
});
