// src/theme.ts - 主题切换统一入口
// 深色/浅色都走这里：改 <html>.dark class + localStorage 持久化。
// 原生窗口底色由 tauri.conf.json 的 backgroundColor 固定（深色），且已移除
// allow-set-background-color 权限（前端不再调用窗口原生 API，最小化 capability 面）。
export type Theme = "dark" | "light";
export const THEME_KEY = "cfw-theme";

export function getTheme(): Theme {
  return localStorage.getItem(THEME_KEY) === "light" ? "light" : "dark";
}

export function setTheme(theme: Theme): void {
  const el = document.documentElement;
  // data-theme 驱动设计系统变量；dark class 供 Element Plus 深色 css-vars 使用。
  el.classList.toggle("dark", theme === "dark");
  el.setAttribute("data-theme", theme);
  localStorage.setItem(THEME_KEY, theme);
}
