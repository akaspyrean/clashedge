import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";

// Vitest 配置（与 vite.config.ts 共享 alias / Vue 插件）。
// 单独一份配置而非 merge vite.config.ts：后者包含 Tauri dev server 相关设置
// （端口、CSP、watch 忽略 src-tauri），对单元测试无意义且会干扰 jsdom 环境。
export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    include: ["src/**/*.spec.ts"],
  },
});
