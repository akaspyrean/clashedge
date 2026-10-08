// src/theme.ts - 主题切换统一入口
// 三态：system（跟随系统）/ light / dark。
// 持久化存原始三态值；实际渲染解析为 light/dark：
// - data-theme 属性驱动设计系统变量；
// - dark class 供 Element Plus 深色 css-vars 使用。
// 原生窗口底色由 tauri.conf.json 的 backgroundColor 固定（深色），且已移除
// allow-set-background-color 权限（前端不再调用窗口原生 API，最小化 capability 面）。
export type Theme = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";
export const THEME_KEY = "cfw-theme";

const prefersDark = window.matchMedia("(prefers-color-scheme: dark)");

/** system 态的跟随监听句柄：切出 system 态时注销。 */
let systemListener: (() => void) | null = null;

/** 将三态值解析为实际生效的 light/dark；system 跟随系统偏好。 */
function resolveTheme(theme: Theme): ResolvedTheme {
  if (theme !== "system") return theme;
  return prefersDark.matches ? "dark" : "light";
}

/** 只应用主题 class/属性到 <html>，不写 localStorage。 */
function applyTheme(theme: Theme): void {
  const el = document.documentElement;
  const resolved = resolveTheme(theme);
  el.classList.toggle("dark", resolved === "dark");
  el.setAttribute("data-theme", resolved);
}

export function getTheme(): Theme {
  const v = localStorage.getItem(THEME_KEY);
  return v === "light" || v === "dark" || v === "system" ? v : "dark";
}

export function setTheme(theme: Theme): void {
  localStorage.setItem(THEME_KEY, theme);
  // 切换前先摘除旧的 system 监听，避免泄漏与重复触发。
  if (systemListener) {
    prefersDark.removeEventListener("change", systemListener);
    systemListener = null;
  }
  if (theme === "system") {
    // WebView2 支持 matchMedia change 事件；系统深浅切换时实时跟随。
    systemListener = () => applyTheme("system");
    prefersDark.addEventListener("change", systemListener);
  }
  applyTheme(theme);
}
