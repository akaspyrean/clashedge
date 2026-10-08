import { describe, expect, it, vi } from "vitest";
import { defineComponent, h } from "vue";
import { mountApp } from "@/test/harness";
import { useAction } from "./useAction";

const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn(), info: vi.fn() }));
vi.mock("element-plus", async (orig) => ({
  ...(await orig<typeof import("element-plus")>()),
  ElMessage: toast,
}));

function setup() {
  let api!: ReturnType<typeof useAction>;
  mountApp(
    defineComponent({
      setup() {
        api = useAction();
        return () => h("div");
      },
    }),
  );
  return api;
}

describe("useAction", () => {
  it("returns the value, toggles busy and shows the success toast", async () => {
    toast.success.mockClear();
    const a = setup();
    const p = a.run(async () => 42, { success: "done" });
    expect(a.busy.value).toBe(true);
    expect(await p).toEqual({ ok: true, value: 42 });
    expect(a.busy.value).toBe(false);
    expect(toast.success).toHaveBeenCalledWith("done");
  });

  it("ignores re-entrant calls while one is in flight", async () => {
    const a = setup();
    let release!: () => void;
    const fn = vi.fn(() => new Promise<void>((r) => (release = r)));
    const first = a.run(fn);
    const second = await a.run(fn);
    expect(second).toEqual({ ok: false });
    expect(fn).toHaveBeenCalledTimes(1);
    release();
    expect((await first).ok).toBe(true);
  });

  it("reports failures through the error toast and resets busy", async () => {
    toast.error.mockClear();
    const a = setup();
    const r = await a.run(async () => {
      throw new Error("boom");
    });
    expect(r).toEqual({ ok: false });
    expect(a.busy.value).toBe(false);
    expect(toast.error).toHaveBeenCalledTimes(1);
  });

  it("silent mode swallows the toast", async () => {
    toast.error.mockClear();
    const a = setup();
    await a.run(async () => Promise.reject(new Error("x")), { silent: true });
    expect(toast.error).not.toHaveBeenCalled();
  });
});
