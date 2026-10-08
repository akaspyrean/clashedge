import { beforeEach, describe, expect, it, vi } from "vitest";

// 与 config.spec.ts 同一模式：mock @tauri-apps/api/core 的 invoke，
// 测试只关心 profiles store 的状态机，不验证后端行为。
const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { setActivePinia, createPinia } from "pinia";
import { useProfilesStore } from "@/stores/profiles";
import type { ProfileInfo } from "@/api/profiles";

function baseProfiles(): ProfileInfo[] {
  return [
    { name: "机场A", path: "机场A.yaml", active: true, url: "https://a.example/sub" },
    { name: "机场B", path: "机场B.yaml", active: false, url: null },
  ];
}

function primeStore(store: ReturnType<typeof useProfilesStore>) {
  store.profiles = JSON.parse(JSON.stringify(baseProfiles()));
}

describe("profiles store state machine", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    invokeMock.mockReset();
  });

  it("list 成功后写入 profiles 并清 loading", async () => {
    const store = useProfilesStore();
    invokeMock.mockResolvedValueOnce(baseProfiles());

    await store.list();

    expect(invokeMock).toHaveBeenCalledWith("list_profiles");
    expect(store.profiles).toHaveLength(2);
    expect(store.loading).toBe(false);
  });

  it("list 失败时兜底为空列表、不抛错、loading 复位", async () => {
    const store = useProfilesStore();
    invokeMock.mockRejectedValueOnce(new Error("boom"));

    await expect(store.list()).resolves.toBeUndefined();
    expect(store.profiles).toEqual([]);
    expect(store.loading).toBe(false);
  });

  it("activeProfile getter 返回激活项", () => {
    const store = useProfilesStore();
    primeStore(store);

    expect(store.activeProfile?.name).toBe("机场A");
  });

  it("create/remove/rename 后都重新 list 刷新", async () => {
    const store = useProfilesStore();
    invokeMock.mockResolvedValue(baseProfiles());

    await store.create("新配置");
    expect(invokeMock).toHaveBeenCalledWith("create_profile", { name: "新配置", content: undefined });

    await store.remove("机场A");
    expect(invokeMock).toHaveBeenCalledWith("delete_profile", { name: "机场A" });

    await store.rename("机场A", "机场C");
    expect(invokeMock).toHaveBeenCalledWith("rename_profile", { oldName: "机场A", newName: "机场C" });

    // 每个命令后都重新拉列表：3 命令 × (命令 + list) = 6 次 invoke
    expect(invokeMock).toHaveBeenCalledTimes(6);
  });

  it("activate 只本地标记 active，不重新拉列表", async () => {
    const store = useProfilesStore();
    primeStore(store);
    invokeMock.mockClear();

    await store.activate("机场B");

    expect(invokeMock).toHaveBeenCalledWith("activate_profile", { name: "机场B" });
    // 本地 map，不触发 list_profiles
    expect(invokeMock).not.toHaveBeenCalledWith("list_profiles");
    expect(store.activeProfile?.name).toBe("机场B");
    const names = store.profiles.filter((p) => p.active).map((p) => p.name);
    expect(names).toEqual(["机场B"]);
  });
});
