import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));

import { freshPinia, mountApp } from "@/test/harness";
import DegradedBanner from "./DegradedBanner.vue";

let degraded = true;
beforeEach(() => {
  degraded = true;
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "get_config_degraded")
      return Promise.resolve({ degraded, backup_file: "C:/d/config.yaml.corrupt-1.bak", message: "m" });
    if (cmd === "confirm_overwrite_corrupt_config") {
      degraded = false;
      return Promise.resolve(undefined);
    }
    if (cmd === "get_config") return Promise.resolve({ mode: "rule" });
    return Promise.resolve(undefined);
  });
});

describe("DegradedBanner", () => {
  it("shows the backup path and an alert role while degraded", async () => {
    const w = mountApp(DegradedBanner, {}, freshPinia());
    await flushPromises();
    expect(w.find("[role=alert]").exists()).toBe(true);
    expect(w.text()).toContain("degraded.backup");
    expect(w.text()).toContain("degraded.confirm");
  });

  it("renders nothing when the config is healthy", async () => {
    degraded = false;
    const w = mountApp(DegradedBanner, {}, freshPinia());
    await flushPromises();
    expect(w.find("[role=alert]").exists()).toBe(false);
  });

  it("confirming calls the backend once and hides the banner", async () => {
    const w = mountApp(DegradedBanner, {}, freshPinia());
    await flushPromises();
    await w.find("button").trigger("click");
    await flushPromises();
    const calls = invokeMock.mock.calls.filter((c) => c[0] === "confirm_overwrite_corrupt_config");
    expect(calls).toHaveLength(1);
    expect(w.find("[role=alert]").exists()).toBe(false);
  });
});
