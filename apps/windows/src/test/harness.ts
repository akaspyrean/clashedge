// src/test/harness.ts - 组件测试的统一挂载环境：Pinia + Element Plus + vue-i18n（空消息表，
// t(key) 回退为 key，断言按 key 匹配）。各视图 / 组件规格共用，避免每个 spec 各搭一套。

import { createPinia, setActivePinia, type Pinia } from "pinia";
import ElementPlus from "element-plus";
import { createI18n } from "vue-i18n";
import { mount, type ComponentMountingOptions } from "@vue/test-utils";
import type { Component } from "vue";

export function testI18n() {
  return createI18n({ legacy: false, locale: "zh-CN", messages: { "zh-CN": {} }, missingWarn: false, fallbackWarn: false });
}

export function freshPinia(): Pinia {
  const pinia = createPinia();
  setActivePinia(pinia);
  return pinia;
}

/** 挂载组件并安装 Pinia / Element Plus / i18n。`$t` 同时以 mocks 提供（模板里的全局注入）。 */
export function mountApp(
  component: Component,
  options: ComponentMountingOptions<Component> = {},
  pinia: Pinia = freshPinia(),
) {
  const { global: g, ...rest } = options as { global?: Record<string, unknown> } & Record<string, unknown>;
  return mount(component, {
    ...(rest as object),
    global: {
      ...(g ?? {}),
      plugins: [pinia, ElementPlus, testI18n(), ...(((g ?? {}).plugins as unknown[]) ?? [])],
      mocks: { $t: (k: string) => k, ...(((g ?? {}).mocks as object) ?? {}) },
    },
  } as never);
}

/** theme.ts 在模块顶层读 matchMedia、部分视图用 ResizeObserver：happy-dom 均缺失。 */
export function stubBrowserApis(vi: { stubGlobal: (k: string, v: unknown) => void }) {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  }));
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
}
