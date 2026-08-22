// src/main.ts - 应用入口
import { createApp } from "vue";
import { createPinia } from "pinia";
import ElementPlus from "element-plus";
import "element-plus/dist/index.css";
import * as ElementPlusIconsVue from "@element-plus/icons-vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import App from "./App.vue";
import router from "./router";
import { setupI18n } from "./i18n";
import { useConfigStore } from "@/stores/config";
import { useCoreStore } from "@/stores/core";
import { useProxyStore } from "@/stores/proxy";
import "./styles.css";
import { getTheme, setTheme } from "./theme";

// R8.3 默认深色；设置页可选 system/light/dark 三态（localStorage cfw-theme 持久化）。
// data-theme 驱动设计系统；dark class 供 Element Plus 深色 css-vars 使用。
// setTheme 会解析 system 态并注册跟随监听，幂等。
setTheme(getTheme());

// 应用生命周期监听器：这些监听跟随应用整个生命周期（非页面级），收集 UnlistenFn
// 便于统一释放；bootstrap 只执行一次，此处由 bootstrapStarted 防重入。
const lifecycleListeners: UnlistenFn[] = [];
let bootstrapStarted = false;

/** 注册一个应用生命周期监听器，失败时记录错误而非静默吞掉。 */
function registerListener(event: string, handler: () => void): void {
  listen(event, handler)
    .then((unlisten) => lifecycleListeners.push(unlisten))
    .catch((e) => console.error(`failed to listen event "${event}"`, e));
}

async function bootstrap() {
  if (bootstrapStarted) return;
  bootstrapStarted = true;
  const app = createApp(App);
  app.use(createPinia());

  // 图标全局注册：模板中可直接使用 <Odometer /> 等组件名。
  for (const [name, comp] of Object.entries(ElementPlusIconsVue)) {
    app.component(name, comp);
  }

  // 等待 i18n 初始化（先读后端配置的语言，再拉取消息表）。
  const i18n = await setupI18n();
  app.use(i18n);

  app.use(ElementPlus);
  app.use(router);
  app.mount("#app");

  // 后端在 mihomo 异常退出时推送 core-status-changed；收到后刷新 store，
  // 保证「界面状态 = 应用状态」（无需等下一次轮询才反映崩溃）。
  // 托盘/UI 发起 restart 时，start() 成功后也会推送 running → 一并刷新代理组
  // （重启后节点列表可能变化，旧选中态也随之失效）。
  registerListener("core-status-changed", () => {
    void useCoreStore().refresh();
    void useProxyStore().loadGroups();
  });

  // 订阅/Profile 激活后，mihomo 可能热重载或重启成功，
  // 但前端代理组列表仍是旧数据 → 监听 profile-activated 主动拉取最新 /proxies。
  registerListener("profile-activated", () => {
    void useProxyStore().loadGroups();
  });

  // 托盘右键修改的配置（代理模式/系统代理/TUN/配置混合）须同步回 UI：
  // 后端统一编排层 apply_* 已持久化并 emit 对应事件，这里监听后重载配置，
  // 保证「界面状态 = 应用状态」（否则托盘改了设置，设置页仍显示旧值）。
  const reloadConfig = () => void useConfigStore().load();

  registerListener("proxy-mode-changed", reloadConfig);
  registerListener("system-proxy-changed", reloadConfig);
  registerListener("tun-mode-changed", reloadConfig);
  registerListener("config-mixin-changed", reloadConfig);

  // 托盘里切换代理组后，前端代理组选中态也要跟随刷新。
  registerListener("proxy-group-changed", () => {
    void useProxyStore().loadGroups();
  });
}

void bootstrap();
