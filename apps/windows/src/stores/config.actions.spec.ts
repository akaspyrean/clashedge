import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { useConfigStore } from "./config";
import type { ClashConfig } from "@/api/config";

function cfg(over: Record<string, unknown> = {}): ClashConfig {
  return {
    mode: "rule",
    "system-proxy": false,
    locale: "zh-CN",
    "mixed-port": 7890,
    tun: { enable: false, stack: "mixed", "auto-route": true, "auto-detect-interface": true },
    ...over,
  } as unknown as ClashConfig;
}

async function primed() {
  const s = useConfigStore();
  invokeMock.mockResolvedValueOnce(cfg());
  await s.load();
  invokeMock.mockReset();
  return s;
}

beforeEach(() => {
  setActivePinia(createPinia());
  invokeMock.mockReset();
});

describe("config store: immediate-apply actions", () => {
  it("setProxyMode updates memory AND baseline, so save() will not resend it", async () => {
    const s = await primed();
    invokeMock.mockResolvedValue(undefined);
    await s.setProxyMode("global");
    expect(invokeMock).toHaveBeenCalledWith("set_proxy_mode", { mode: "global" });
    expect(s.config?.mode).toBe("global");
    expect(s.baseline?.mode).toBe("global");
    expect(s.diffFromBaseline()).toEqual({});
  });

  it("setSystemProxy keeps the old value when the backend refuses", async () => {
    const s = await primed();
    invokeMock.mockRejectedValue("denied");
    await expect(s.setSystemProxy(true)).rejects.toBe("denied");
    expect(s.config?.["system-proxy"]).toBe(false);
  });

  it("setTunMode reloads the real state from the backend on failure", async () => {
    const s = await primed();
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "set_tun_mode" ? Promise.reject("no admin") : Promise.resolve(cfg()),
    );
    await expect(s.setTunMode(true)).rejects.toBe("no admin");
    expect(invokeMock).toHaveBeenCalledWith("get_config");
    expect(s.config?.tun.enable).toBe(false);
  });

  it("importFromFile returns false when the user cancels", async () => {
    const s = await primed();
    invokeMock.mockResolvedValue(null);
    expect(await s.importFromFile()).toBe(false);
    expect(invokeMock).not.toHaveBeenCalledWith("import_config", expect.anything());
  });

  it("confirmOverwriteCorrupt leaves degraded mode", async () => {
    const s = await primed();
    s.degraded = true;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_config_degraded") return Promise.resolve({ degraded: false, backup_file: null, message: "" });
      if (cmd === "get_config") return Promise.resolve(cfg());
      return Promise.resolve(undefined);
    });
    await s.confirmOverwriteCorrupt();
    expect(invokeMock).toHaveBeenCalledWith("confirm_overwrite_corrupt_config");
    expect(s.degraded).toBe(false);
  });
});
