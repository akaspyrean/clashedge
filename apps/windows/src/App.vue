<!-- src/App.vue - 应用外壳：自绘标题栏 + 侧边栏导航 + 内容区
     深色主题；Element Plus 语言包随配置语言响应式切换。
     主窗口已设置 set_decorations(false)，故需自绘标题栏提供窗口控制。 -->
<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  House,
  Globe,
  Layers,
  ListFilter,
  SlidersHorizontal,
  Minus,
  Maximize,
  Copy,
  X,
} from "lucide-vue-next";
import { ElMessage } from "element-plus";
import zhCn from "element-plus/es/locale/lang/zh-cn";
import en from "element-plus/es/locale/lang/en";
import { useAppStore } from "@/stores/app";
import { useConfigStore } from "@/stores/config";
import { useCoreStore } from "@/stores/core";
import { utilApi } from "@/api/util";
import DegradedBanner from "@/components/DegradedBanner.vue";
import StatusBanner from "@/components/StatusBanner.vue";
import { getTheme, setTheme } from "./theme";

const appStore = useAppStore();
const configStore = useConfigStore();
const coreStore = useCoreStore();
const route = useRoute();
const { t } = useI18n();

onMounted(async () => {
  setTheme(getTheme()); // 同步主题 class 与 localStorage，幂等。

  // 初始化失败不阻塞窗口显示：
  // - 配置加载失败用默认值兜底（各 getter 已有默认值），仅记录日志；
  // - appStore.init() / coreStore.refresh() 任一失败也不中断窗口显示流程。
  try {
    await configStore.load();
  } catch (e) {
    console.error("加载配置失败，使用默认值兜底", e);
  }
  try {
    await Promise.all([appStore.init(), coreStore.refresh()]);
  } catch (e) {
    console.error("应用/核心状态初始化失败，继续显示窗口", e);
  }

  // 手动启动显示主窗口；自启动（--clash-edge-autostart）只驻留托盘不弹窗。
  // 等初始化完成后 show，避免黑色闪屏。isAutostart() IPC 失败时按「非自启动」处理，
  // 保证窗口照常显示（浏览器 dev 环境下 win.show 不可用，由外层 catch 忽略）。
  try {
    const autostart = await utilApi.isAutostart().catch(() => false);
    if (!autostart) {
      await win.show();
    }
  } catch (e) {
    // 非 Tauri 环境（浏览器 dev）或权限缺失时忽略。
    console.error("显示主窗口失败（非 Tauri 环境可忽略）", e);
  }
  void syncMaximized();
  try {
    unlistenResized = await win.onResized(() => {
      void syncMaximized();
    });
  } catch {
    // 监听 API 不可用时降级为轮询。
    pollTimer = window.setInterval(() => {
      void syncMaximized();
    }, 500);
  }
  narrowMql.addEventListener("change", onNarrowChange);
});

onUnmounted(() => {
  unlistenResized?.();
  if (pollTimer !== undefined) window.clearInterval(pollTimer);
  narrowMql.removeEventListener("change", onNarrowChange);
});

const elLocale = computed(() =>
  (configStore.locale ?? "zh-CN").startsWith("zh") ? zhCn : en
);

// 核心错误状态（如端口被占用导致代理监听失败）：顶部横幅展示真实原因，
// 避免「界面看似运行、实际代理端口已死」的假象。
const coreError = computed(() => {
  const s = coreStore.status?.status ?? "";
  return s.startsWith("error:") ? s.slice("error:".length).trim() : "";
});

// 无障碍：路由切换后把焦点移到主内容区并更新窗口标题——键盘 / 读屏用户无需重新 Tab
// 穿过侧栏才能到达新页面内容；文档语言随界面语言更新（读屏器据此选发音）。
const mainEl = ref<{ $el: HTMLElement } | null>(null);

function focusMain() {
  mainEl.value?.$el?.focus({ preventScroll: true });
}

watch(
  () => route.path,
  async () => {
    await nextTick();
    const key = menuItems.find((m) => route.path.startsWith(m.path))?.key;
    document.title = key ? `${t(key)} - ClashEdge` : "ClashEdge";
    focusMain();
  },
);

watch(
  () => configStore.locale,
  (loc) => {
    document.documentElement.lang = loc ?? "zh-CN";
  },
  { immediate: true },
);

const menuItems = [
  { path: "/dashboard", key: "nav.dashboard", icon: House },
  { path: "/proxies", key: "nav.proxies", icon: Globe },
  { path: "/profiles", key: "nav.profiles", icon: Layers },
  { path: "/rules", key: "nav.rules", icon: ListFilter },
  { path: "/settings", key: "nav.settings", icon: SlidersHorizontal },
];

// ---- 自绘标题栏 ----
const win = getCurrentWindow();
const isMaximized = ref(false);
let unlistenResized: (() => void) | undefined;
let pollTimer: number | undefined;

async function syncMaximized() {
  try {
    isMaximized.value = await win.isMaximized();
  } catch {
    // 非 Tauri 环境（浏览器 dev）下忽略。
  }
}

async function onMinimize() {
  try {
    await win.minimize();
  } catch {
    // ignore
  }
}

async function onToggleMaximize() {
  try {
    await win.toggleMaximize();
  } catch {
    // ignore
  }
}

let closeHintShown = false;

async function onClose() {
  // 关闭按钮实际触发的是「最小化到托盘」（后端 CloseRequested 拦截为 hide）。
  // 首次点击提示用户应用仍在托盘运行，避免误以为已退出。
  if (!closeHintShown) {
    closeHintShown = true;
    ElMessage.info(t("titlebar.minimized_to_tray"));
  }
  try {
    await win.close();
  } catch {
    // ignore
  }
}

// ---- 响应式侧栏：窗口 < 750px 收为 icon-only（文字语义由 title 提示保留）----
const narrowMql = window.matchMedia("(max-width: 749px)");
const isNarrow = ref(narrowMql.matches);

function onNarrowChange(e: MediaQueryListEvent) {
  isNarrow.value = e.matches;
}
</script>

<template>
  <el-config-provider :locale="elLocale">
    <div class="frame">
      <button type="button" class="skip-link" @click="focusMain">{{ $t("a11y.skip_to_content") }}</button>
      <header class="titlebar">
        <div class="titlebar-drag" data-tauri-drag-region>
          <h1 class="titlebar-title" data-tauri-drag-region>ClashEdge</h1>
        </div>
        <div class="titlebar-controls">
          <button
            type="button"
            class="tb-btn"
            :title="$t('titlebar.minimize')"
            :aria-label="$t('titlebar.minimize')"
            @click="onMinimize"
          >
            <Minus :size="14" :stroke-width="1.75" />
          </button>
          <button
            type="button"
            class="tb-btn"
            :title="isMaximized ? $t('titlebar.restore') : $t('titlebar.maximize')"
            :aria-label="isMaximized ? $t('titlebar.restore') : $t('titlebar.maximize')"
            @click="onToggleMaximize"
          >
            <Maximize v-if="!isMaximized" :size="13" :stroke-width="1.75" />
            <Copy v-else :size="13" :stroke-width="1.75" />
          </button>
          <button
            type="button"
            class="tb-btn tb-close"
            :title="$t('titlebar.close')"
            :aria-label="$t('titlebar.close')"
            @click="onClose"
          >
            <X :size="14" :stroke-width="1.75" />
          </button>
        </div>
      </header>

      <el-container class="app-shell">
        <el-aside :width="isNarrow ? '64px' : '184px'" class="app-aside" :class="{ narrow: isNarrow }">
          <nav :aria-label="$t('a11y.main_navigation')">
          <el-menu :default-active="route.path" router class="app-menu">
            <el-menu-item
              v-for="item in menuItems"
              :key="item.path"
              :index="item.path"
              :title="$t(item.key)"
            >
              <el-icon><component :is="item.icon" :size="20" :stroke-width="1.75" /></el-icon>
              <span class="menu-label">{{ $t(item.key) }}</span>
            </el-menu-item>
          </el-menu>
          </nav>
          <!-- 次级功能：连接 / 日志，非核心任务入口 -->
          <nav class="secondary-nav" :aria-label="$t('a11y.secondary_navigation')">
            <router-link to="/connections" class="secondary-link">{{ $t("nav.connections") }}</router-link>
            <router-link to="/logs" class="secondary-link">{{ $t("nav.logs") }}</router-link>
          </nav>
        </el-aside>
        <el-main id="main-content" ref="mainEl" class="app-main" tabindex="-1">
          <DegradedBanner />
          <StatusBanner v-if="coreError" :title="coreError" type="error" />
          <router-view />
        </el-main>
      </el-container>
    </div>
  </el-config-provider>
</template>

<style scoped>
.frame {
  height: 100vh;
  display: flex;
  flex-direction: column;
  background-color: var(--ce-bg-page);
}

.titlebar {
  flex: none;
  height: 36px;
  display: flex;
  align-items: stretch;
  background-color: var(--ce-bg-surface);
  border-bottom: 1px solid var(--ce-border);
  user-select: none;
  flex-shrink: 0;
}

.titlebar-drag {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  padding: 0 14px;
  cursor: default;
}

.titlebar-title {
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.3px;
  color: var(--ce-text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.titlebar-controls {
  flex: none;
  display: flex;
  align-items: stretch;
}

.tb-btn {
  width: 46px;
  border: none;
  margin: 0;
  padding: 0;
  background: transparent;
  color: var(--ce-text-secondary);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition:
    background-color 0.15s ease,
    color 0.15s ease;
}

.tb-btn:hover {
  background-color: var(--ce-fill-hover);
  color: var(--ce-text-primary);
}

.tb-close:hover {
  background-color: var(--ce-danger-fg);
  color: var(--ce-on-accent);
}

/* 键盘可达性：标题栏按钮聚焦时显示清晰焦点环。 */
.tb-btn:focus-visible {
  outline: 2px solid var(--ce-accent-fg);
  outline-offset: -2px;
}

/* 响应式：窗口 < 860px 时侧栏收为 icon-only，
   文字语义保留在 el-menu-item 的 title 提示上。 */
.app-aside.narrow :deep(.menu-label) {
  display: none;
}

.app-aside.narrow :deep(.app-menu .el-menu-item) {
  justify-content: center;
  padding: 0 !important;
}

/* 覆盖全局 .app-shell { height: 100vh }，改为在标题栏下方弹性填满。 */
.app-shell {
  flex: 1;
  min-height: 0;
  height: auto;
}

.titlebar-title {
  margin: 0;
}

/* 次级导航：连接/日志的小字链接 */
.secondary-nav {
  margin-top: auto;
  padding: 0 20px 12px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.secondary-link {
  font-size: 12px;
  line-height: 28px;
  color: var(--ce-text-tertiary);
  text-decoration: none;
  transition: color 0.15s ease;
}

.secondary-link:hover {
  color: var(--ce-text-secondary);
}

.secondary-link.router-link-active {
  color: var(--ce-accent-fg);
}

.app-aside.narrow .secondary-nav {
  display: none;
}

/* 跳转到主内容：平时移出视口，键盘聚焦时出现。 */
.skip-link {
  position: absolute;
  left: 8px;
  top: -48px;
  z-index: 3000;
  padding: 8px 14px;
  border-radius: var(--ce-radius-md);
  border: 1px solid var(--ce-accent-fg);
  background: var(--ce-bg-surface);
  color: var(--ce-text-primary);
  font-size: 13px;
}

.skip-link:focus-visible {
  top: 8px;
}

.app-main:focus {
  outline: none;
}
</style>
