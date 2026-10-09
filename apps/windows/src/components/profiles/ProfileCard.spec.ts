import { describe, expect, it } from "vitest";
import { mountApp } from "@/test/harness";
import ProfileCard from "./ProfileCard.vue";

const sub = {
  name: "air",
  path: "air.yaml",
  active: false,
  url: "https://example.com/x",
  userinfo: "upload=1024; download=2048; total=1073741824; expire=0",
  user_agent: null,
};

const labels = (w: ReturnType<typeof mountApp>) => w.findAll("button").map((b) => b.attributes("aria-label") ?? "");

describe("ProfileCard", () => {
  it("names every button with the profile it acts on", () => {
    const w = mountApp(ProfileCard, { props: { profile: sub } });
    expect(labels(w)).toContain("profiles.activate: air");
    // 图标按钮（更多）也必须有名称，而不是只有 title
    expect(labels(w)).toContain("profiles.more: air");
  });

  it("offers exactly one primary action: use → update → raw edit", () => {
    const inactive = mountApp(ProfileCard, { props: { profile: sub } });
    expect(labels(inactive).filter((l) => l !== "profiles.more: air")).toEqual(["profiles.activate: air"]);

    const activeSub = mountApp(ProfileCard, { props: { profile: { ...sub, active: true } } });
    expect(labels(activeSub)).toContain("profiles.update: air");
    expect(labels(activeSub).some((l) => l.startsWith("profiles.activate"))).toBe(false);

    const activeLocal = mountApp(ProfileCard, { props: { profile: { ...sub, active: true, url: null } } });
    expect(labels(activeLocal)).toContain("profiles.raw_edit: air");
    expect(labels(activeLocal).some((l) => l.startsWith("profiles.update"))).toBe(false);
  });

  it("emits activate / refresh / edit from the primary action", async () => {
    const act = mountApp(ProfileCard, { props: { profile: sub } });
    await act.findAll("button").find((b) => b.attributes("aria-label") === "profiles.activate: air")!.trigger("click");
    expect(act.emitted("activate")).toHaveLength(1);

    const ref = mountApp(ProfileCard, { props: { profile: { ...sub, active: true } } });
    await ref.findAll("button").find((b) => b.attributes("aria-label") === "profiles.update: air")!.trigger("click");
    expect(ref.emitted("refresh")).toHaveLength(1);

    const edit = mountApp(ProfileCard, { props: { profile: { ...sub, active: true, url: null } } });
    await edit.findAll("button").find((b) => b.attributes("aria-label") === "profiles.raw_edit: air")!.trigger("click");
    expect(edit.emitted("edit")).toHaveLength(1);
  });

  it("shows url in mono, or a local-file label for local profiles", () => {
    expect(mountApp(ProfileCard, { props: { profile: sub } }).find(".url").text()).toBe("https://example.com/x");
    expect(mountApp(ProfileCard, { props: { profile: { ...sub, url: null } } }).find(".url").text()).toBe("profiles.local_file");
  });

  it("renders a usage bar, traffic text and never-expires", () => {
    const w = mountApp(ProfileCard, { props: { profile: sub } });
    expect(w.find('[role="progressbar"]').exists()).toBe(true);
    expect(w.text()).toContain("profiles.traffic:");
    expect(w.text()).toContain("profiles.traffic_never");
  });

  it("hides the usage block when the subscription reports no traffic info", () => {
    const w = mountApp(ProfileCard, { props: { profile: { ...sub, userinfo: null } } });
    expect(w.find(".usage").exists()).toBe(false);
  });

  it("status tags combine colour with text: active, expiring, expired, failed", () => {
    const day = 86_400;
    const now = Math.floor(Date.now() / 1000);
    const mk = (expireOffsetDays: number, extra: object = {}) =>
      mountApp(ProfileCard, {
        props: {
          profile: { ...sub, active: true, userinfo: `upload=0; download=0; total=100; expire=${now + expireOffsetDays * day}` },
          ...extra,
        },
      });
    const tagTexts = (w: ReturnType<typeof mountApp>) => w.findAll(".tag").map((t) => t.text());
    expect(tagTexts(mk(30))).toEqual(["profiles.active"]);
    expect(tagTexts(mk(3))).toEqual(["profiles.active", "profiles.status_expiring"]);
    expect(tagTexts(mk(-2))).toEqual(["profiles.active", "profiles.status_expired"]);
    expect(tagTexts(mk(30, { failed: true }))).toEqual(["profiles.active", "profiles.status_failed"]);
  });
});
