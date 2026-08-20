import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [vue()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    // dev-only CSP（仅注入 dev server 响应头，不写入打包产物；生产 CSP 由
    // src-tauri/tauri.conf.json 注入，避免 meta CSP 与 Tauri 注入的 CSP 交集收窄）。
    headers: {
      "Content-Security-Policy":
        "default-src 'self'; script-src 'self' 'unsafe-inline' http://localhost:1420; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; connect-src 'self' ws://localhost:1420 http://localhost:1420; object-src 'none'; base-uri 'self'; frame-src 'none'",
    },
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
  resolve: {
    alias: {
      // 注意：不要用 `new URL(...).pathname` —— 含空格路径会被 URL 编码成 %20。
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
}));
