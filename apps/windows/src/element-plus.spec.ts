// 回归：按需注册（element-plus.ts）漏掉某个 <el-xxx> 时，运行时只会在控制台出一条
// "Failed to resolve component" 警告，界面上该控件静默消失（曾经：设置页「主题」整行只剩文字，
// 因为 ElRadioGroup / ElRadioButton 是 noop-install 子组件，必须通过 ElRadio 安装）。
// 这里静态扫描所有 .vue 模板里用到的 <el-*> 标签，逐个确认已被注册。
import { describe, expect, it } from "vitest";
import { createApp } from "vue";
import { installElementPlus } from "./element-plus";

const sources = import.meta.glob("./**/*.vue", { query: "?raw", import: "default", eager: true }) as Record<
  string,
  string
>;

const pascal = (tag: string) =>
  tag
    .split("-")
    .map((p) => p.charAt(0).toUpperCase() + p.slice(1))
    .join("");

describe("Element Plus on-demand registration", () => {
  it("registers every <el-*> component used in any template", () => {
    const used = new Map<string, string>(); // tag -> first file
    for (const [file, src] of Object.entries(sources)) {
      for (const m of src.matchAll(/<(el-[a-z0-9-]+)/g)) {
        if (!used.has(m[1])) used.set(m[1], file);
      }
    }
    expect(used.size).toBeGreaterThan(20);

    const app = createApp({});
    installElementPlus(app);
    const missing = [...used.entries()]
      .filter(([tag]) => !app.component(pascal(tag)))
      .map(([tag, file]) => `${tag} (${file})`);
    expect(missing, `unregistered: ${missing.join(", ")}`).toEqual([]);
  });
});
