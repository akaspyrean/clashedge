// src/router/index.ts
// 路由：Tauri 桌面应用使用 hash 历史模式（避免文件协议下的路径问题）。

import { createRouter, createWebHashHistory } from "vue-router";

const routes = [
  { path: "/", redirect: "/dashboard" },
  {
    path: "/dashboard",
    name: "dashboard",
    component: () => import("@/views/DashboardView.vue"),
  },
  {
    path: "/proxies",
    name: "proxies",
    component: () => import("@/views/ProxiesView.vue"),
  },
  {
    path: "/profiles",
    name: "profiles",
    component: () => import("@/views/ProfilesView.vue"),
  },
  {
    path: "/connections",
    name: "connections",
    component: () => import("@/views/ConnectionsView.vue"),
  },
  {
    path: "/logs",
    name: "logs",
    component: () => import("@/views/LogsView.vue"),
  },
  {
    path: "/settings",
    name: "settings",
    component: () => import("@/views/SettingsView.vue"),
  },
];

export default createRouter({
  history: createWebHashHistory(),
  routes,
});
