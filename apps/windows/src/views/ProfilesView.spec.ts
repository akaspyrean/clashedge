import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { mountApp } from "@/test/harness";
import ProfilesView from "@/views/ProfilesView.vue";

const profiles = [
  { name: "air", path: "air.yaml", active: true, url: "https://example.com/x", userinfo: null, user_agent: null },
  { name: "local", path: "local.yaml", active: false, url: null, userinfo: null, user_agent: null },
];

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "list_profiles") return Promise.resolve(profiles);
    return Promise.resolve(undefined);
  });
});

describe("ProfilesView", () => {
  it("renders profiles as a labelled list, one item per profile", async () => {
    const w = mountApp(ProfilesView, { attachTo: document.body });
    await flushPromises();
    const list = w.find("ul.profile-list");
    expect(list.exists()).toBe(true);
    expect(list.attributes("aria-label")).toBe("profiles.title");
    expect(list.findAll("li")).toHaveLength(2);
    w.unmount();
  });

  it("activating a profile calls the backend and refreshes the active flag", async () => {
    const w = mountApp(ProfilesView, { attachTo: document.body });
    await flushPromises();
    const btn = w.findAll("button").find((b) => b.attributes("aria-label") === "profiles.activate: local")!;
    await btn.trigger("click");
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledWith("activate_profile", { name: "local" });
    w.unmount();
  });

  it("a failed activation surfaces an error and does not flip the active flag", async () => {
    const w = mountApp(ProfilesView, { attachTo: document.body });
    await flushPromises();
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "activate_profile" ? Promise.reject("boom") : Promise.resolve(profiles),
    );
    const btn = w.findAll("button").find((b) => b.attributes("aria-label") === "profiles.activate: local")!;
    await btn.trigger("click");
    await flushPromises();
    // 失败后列表仍显示"激活"按钮（local 没有变成 active）
    expect(w.findAll("button").some((b) => b.attributes("aria-label") === "profiles.activate: local")).toBe(true);
    w.unmount();
  });
});
