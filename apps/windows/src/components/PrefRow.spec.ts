import { describe, expect, it } from "vitest";
import { h } from "vue";
import { mountApp } from "@/test/harness";
import PrefRow from "./PrefRow.vue";

describe("PrefRow accessibility", () => {
  it("is a group named by its visible title", () => {
    const w = mountApp(PrefRow, { props: { title: "语言" } });
    const row = w.find(".pref-row");
    expect(row.attributes("role")).toBe("group");
    const labelId = row.attributes("aria-labelledby")!;
    expect(w.find(`[id="${labelId}"]`).text()).toBe("语言");
  });

  it("hands the title to the control so it gets an accessible name", () => {
    const w = mountApp(PrefRow, {
      props: { title: "开机自启", hint: "静默启动" },
      slots: {
        default: ({ label, hintId }: { label: string; hintId?: string }) =>
          h("button", { "aria-label": label, "aria-describedby": hintId }, "x"),
      },
    });
    const btn = w.find("button");
    expect(btn.attributes("aria-label")).toBe("开机自启");
    const describedBy = btn.attributes("aria-describedby")!;
    expect(w.find(`[id="${describedBy}"]`).text()).toBe("静默启动");
  });

  it("does not reference a hint element when there is no hint", () => {
    const w = mountApp(PrefRow, {
      props: { title: "t" },
      slots: { default: ({ hintId }: { hintId?: string }) => h("i", { "data-hint": String(hintId) }) },
    });
    expect(w.find("i").attributes("data-hint")).toBe("undefined");
  });
});
