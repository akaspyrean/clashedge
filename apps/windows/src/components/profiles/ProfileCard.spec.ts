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

describe("ProfileCard", () => {
  it("names every action button with the profile it acts on", () => {
    const w = mountApp(ProfileCard, { props: { profile: sub } });
    const labels = w.findAll("button").map((b) => b.attributes("aria-label"));
    expect(labels).toContain("profiles.activate: air");
    expect(labels).toContain("profiles.update: air");
    // 图标按钮（更多）也必须有名称，而不是只有 title
    expect(labels).toContain("profiles.more: air");
  });

  it("hides activate for the active profile and update for local ones", () => {
    const w = mountApp(ProfileCard, { props: { profile: { ...sub, active: true, url: null } } });
    const labels = w.findAll("button").map((b) => b.attributes("aria-label") ?? "");
    expect(labels.some((l) => l.startsWith("profiles.activate"))).toBe(false);
    expect(labels.some((l) => l.startsWith("profiles.update"))).toBe(false);
  });

  it("renders traffic text and never-expires", () => {
    const w = mountApp(ProfileCard, { props: { profile: sub } });
    expect(w.text()).toContain("profiles.traffic:");
    expect(w.text()).toContain("profiles.traffic_never");
  });

  it("emits activate and refresh", async () => {
    const w = mountApp(ProfileCard, { props: { profile: sub } });
    const btn = (prefix: string) =>
      w.findAll("button").find((b) => (b.attributes("aria-label") ?? "").startsWith(prefix))!;
    await btn("profiles.activate").trigger("click");
    await btn("profiles.update").trigger("click");
    expect(w.emitted("activate")).toHaveLength(1);
    expect(w.emitted("refresh")).toHaveLength(1);
  });
});
