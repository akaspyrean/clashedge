import { describe, expect, it } from "vitest";
import { mountApp } from "@/test/harness";
import CeStatusCore from "./CeStatusCore.vue";

const mountCore = (state: string) =>
  mountApp(CeStatusCore as never, { props: { state }, slots: { default: '<span class="extra">v1</span>' } });

describe("CeStatusCore", () => {
  it.each([
    ["stopped", "ui.core.aria_start"],
    ["running", "ui.core.aria_stop"],
    ["starting", "ui.core.aria_starting"],
    ["error", "ui.core.aria_error"],
  ])("labels the %s state with a verb that matches the action", (state, label) => {
    const w = mountCore(state);
    expect(w.find("button").attributes("aria-label")).toBe(label);
    expect(w.find("button").classes()).toContain(`is-${state}`);
    expect(w.find('[role="status"]').text()).toBe(`ui.core.${state}`);
  });

  it("disables and marks the button busy while starting", () => {
    const b = mountCore("starting").find("button");
    expect(b.attributes("disabled")).toBeDefined();
    expect(b.attributes("aria-busy")).toBe("true");
  });

  it("emits toggle on click and renders the slot", async () => {
    const w = mountCore("stopped");
    await w.find("button").trigger("click");
    expect(w.emitted("toggle")).toHaveLength(1);
    expect(w.find(".extra").exists()).toBe(true);
  });
});
