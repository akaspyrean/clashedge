import { describe, expect, it } from "vitest";
import { mountApp } from "@/test/harness";
import CeSegmented from "./CeSegmented.vue";

const options = [
  { value: "rule", label: "规则" },
  { value: "global", label: "全局" },
  { value: "direct", label: "直连" },
];

function mountSeg(modelValue = "rule") {
  const w: { setProps: (p: object) => Promise<void> } = mountApp(CeSegmented as never, {
    props: {
      modelValue,
      options,
      ariaLabel: "代理模式",
      // 模拟 v-model：把新值回灌，键盘导航才能连续进行。
      "onUpdate:modelValue": (v: string) => w.setProps({ modelValue: v }),
    },
    attachTo: document.body,
  }) as never;
  return w as ReturnType<typeof mountApp>;
}

describe("CeSegmented", () => {
  it("renders a radiogroup with the selected radio checked and roving tabindex", () => {
    const w = mountSeg("global");
    expect(w.attributes("role")).toBe("radiogroup");
    expect(w.attributes("aria-label")).toBe("代理模式");
    const radios = w.findAll('[role="radio"]');
    expect(radios.map((r) => r.attributes("aria-checked"))).toEqual(["false", "true", "false"]);
    expect(radios.map((r) => r.attributes("tabindex"))).toEqual(["-1", "0", "-1"]);
    w.unmount();
  });

  it("emits update:modelValue on click, but not for the current value", async () => {
    const w = mountSeg("rule");
    await w.findAll("button")[2].trigger("click");
    await w.findAll("button")[2].trigger("click"); // 已是当前值：不重复触发
    expect(w.emitted("update:modelValue")).toEqual([["direct"]]);
    w.unmount();
  });

  it("moves selection with arrow keys, wraps around, and supports Home / End", async () => {
    const w = mountSeg("rule");
    const key = (i: number, k: string) => w.findAll("button")[i].trigger("keydown", { key: k });
    await key(0, "ArrowRight"); // rule -> global
    await key(1, "ArrowLeft"); // global -> rule
    await key(0, "ArrowLeft"); // rule -> wraps to direct
    await key(2, "Home"); // direct -> rule
    await key(0, "End"); // rule -> direct
    expect(w.emitted("update:modelValue")).toEqual([["global"], ["rule"], ["direct"], ["rule"], ["direct"]]);
    w.unmount();
  });

  it("skips disabled options when navigating", async () => {
    const w = mountApp(CeSegmented as never, {
      props: {
        modelValue: "rule",
        options: [options[0], { ...options[1], disabled: true }, options[2]],
        ariaLabel: "x",
      },
    });
    await w.findAll("button")[0].trigger("keydown", { key: "ArrowRight" });
    expect(w.emitted("update:modelValue")).toEqual([["direct"]]);
  });
});
