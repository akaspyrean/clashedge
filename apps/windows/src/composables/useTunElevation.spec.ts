import { describe, expect, it, vi } from "vitest";
import { defineComponent, h } from "vue";
import { mountApp } from "@/test/harness";
import { isTunElevationError, useTunElevation } from "./useTunElevation";

const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn(), info: vi.fn() }));
vi.mock("element-plus", async (orig) => ({
  ...(await orig<typeof import("element-plus")>()),
  ElMessage: toast,
}));

const confirmMock = vi.hoisted(() => vi.fn());
vi.mock("./useConfirm", () => ({ useConfirm: () => confirmMock }));

const relaunchMock = vi.hoisted(() => vi.fn());
vi.mock("@/api/proxy", () => ({ proxyApi: { relaunchForTun: relaunchMock } }));

function setup() {
  let api!: ReturnType<typeof useTunElevation>;
  mountApp(
    defineComponent({
      setup() {
        api = useTunElevation();
        return () => h("div");
      },
    }),
  );
  return api;
}

describe("isTunElevationError", () => {
  it("matches only the backend permission message", () => {
    expect(isTunElevationError("开启 TUN 需要管理员权限：Windows 要求以管理员身份运行")).toBe(true);
    expect(isTunElevationError("boom")).toBe(false);
    expect(isTunElevationError(new Error("port bind failed"))).toBe(false);
  });
});

describe("useTunElevation.handleError", () => {
  it("ignores unrelated errors so the default toast still fires upstream", async () => {
    confirmMock.mockClear();
    const a = setup();
    expect(await a.handleError(new Error("boom"))).toBe(false);
    expect(confirmMock).not.toHaveBeenCalled();
    expect(relaunchMock).not.toHaveBeenCalled();
  });

  it("asks for confirmation and relaunches when the user agrees", async () => {
    confirmMock.mockClear();
    relaunchMock.mockClear();
    confirmMock.mockResolvedValue(true);
    const a = setup();
    expect(await a.handleError("开启 TUN 需要管理员权限")).toBe(true);
    expect(confirmMock).toHaveBeenCalledTimes(1);
    expect(relaunchMock).toHaveBeenCalledTimes(1);
  });

  it("stays silent when the user cancels the confirmation", async () => {
    confirmMock.mockClear();
    relaunchMock.mockClear();
    toast.error.mockClear();
    confirmMock.mockResolvedValue(false);
    const a = setup();
    expect(await a.handleError("开启 TUN 需要管理员权限")).toBe(true);
    expect(relaunchMock).not.toHaveBeenCalled();
    expect(toast.error).not.toHaveBeenCalled();
  });

  it("surfaces the UAC rejection (relaunch rejects)", async () => {
    confirmMock.mockClear();
    toast.error.mockClear();
    relaunchMock.mockRejectedValue("已取消管理员授权，未重启。");
    confirmMock.mockResolvedValue(true);
    const a = setup();
    expect(await a.handleError("开启 TUN 需要管理员权限")).toBe(true);
    expect(toast.error).toHaveBeenCalledWith("已取消管理员授权，未重启。");
  });
});
