import { describe, expect, it, vi } from "vitest";
import { defineComponent, h } from "vue";
import { flushPromises } from "@vue/test-utils";

const handlers: Record<string, (e: { payload: unknown }) => void> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = cb;
    return Promise.resolve(() => {});
  },
}));
const invokeMock = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { mountApp } from "@/test/harness";
import { levelClass, MAX_LOG_LINES, useLogStream } from "./useLogStream";

function setup() {
  let api!: ReturnType<typeof useLogStream>;
  const w = mountApp(
    defineComponent({
      setup() {
        api = useLogStream();
        return () => h("div");
      },
    }),
  );
  return { api, w };
}

describe("useLogStream", () => {
  it("starts the backend stream on mount and stops it on unmount", async () => {
    invokeMock.mockClear();
    const { w } = setup();
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("start_log_stream");
    w.unmount();
    expect(invokeMock).toHaveBeenCalledWith("stop_log_stream");
  });

  it("keeps a bounded ring buffer and reports connection state", async () => {
    const { api } = setup();
    await flushPromises();
    for (let i = 0; i < MAX_LOG_LINES + 50; i++) {
      handlers["log-line"]({ payload: { level: "info", message: `m${i}` } });
    }
    expect(api.entries.value).toHaveLength(MAX_LOG_LINES);
    expect(api.entries.value[0].message).toBe("m50");
    expect(api.connected.value).toBe(true);
    expect(api.statusKey.value).toBe("logs.connected");
    handlers["log-error"]({ payload: { error: "down" } });
    expect(api.connected.value).toBe(false);
    expect(api.statusKey.value).toBe("logs.disconnected");
  });

  it("maps levels to style classes", () => {
    expect(levelClass("error")).toBe("lv-error");
    expect(levelClass("warning")).toBe("lv-warning");
    expect(levelClass("debug")).toBe("lv-debug");
    expect(levelClass("anything")).toBe("lv-info");
  });

  it("stamps each line with a local HH:MM:SS time", async () => {
    const { api } = setup();
    await flushPromises();
    api.clear();
    handlers["log-line"]({ payload: { level: "info", message: "x" } });
    expect(api.entries.value[0].time).toMatch(/^\d{2}:\d{2}:\d{2}$/);
  });

  it("pause holds new lines back and resume merges them in order", async () => {
    const { api } = setup();
    await flushPromises();
    api.clear();
    handlers["log-line"]({ payload: { level: "info", message: "a" } });
    api.pause();
    handlers["log-line"]({ payload: { level: "warning", message: "b" } });
    handlers["log-line"]({ payload: { level: "error", message: "c" } });
    expect(api.entries.value.map((e) => e.message)).toEqual(["a"]);
    api.resume();
    expect(api.entries.value.map((e) => e.message)).toEqual(["a", "b", "c"]);
    expect(api.paused.value).toBe(false);
  });

  it("clear also drops lines buffered while paused", async () => {
    const { api } = setup();
    await flushPromises();
    api.clear();
    api.pause();
    handlers["log-line"]({ payload: { level: "info", message: "held" } });
    api.clear();
    api.resume();
    expect(api.entries.value).toEqual([]);
  });
});
