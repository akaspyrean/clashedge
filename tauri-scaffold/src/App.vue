<!-- src/App.vue - 应用外壳：自绘标题栏 + 侧边栏导航 + 内容区
     深色主题；Element Plus 语言包随配置语言响应式切换。
     主窗口已设置 set_decorations(false)，故需自绘标题栏提供窗口控制。 -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, FullScreen, CopyDocument, Close } from "@element-plus/icons-vue";
import { ElMessage } from "element-plus";
import zhCn from "element-plus/es/locale/lang/zh-cn";
import en from "element-plus/es/locale/lang/en";
import { useAppStore } from "@/stores/app";
import { useConfigStore } from "@/stores/config";
import { useCoreStore } from "@/stores/core";
import { utilApi } from "@/api/util";
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

const menuItems = [
  { path: "/dashboard", key: "nav.dashboard", icon: "Odometer" },
  { path: "/proxies", key: "nav.proxies", icon: "SetUp" },
  { path: "/profiles", key: "nav.profiles", icon: "Document" },
  { path: "/connections", key: "nav.connections", icon: "Link" },
  { path: "/logs", key: "nav.logs", icon: "Tickets" },
  { path: "/settings", key: "nav.settings", icon: "Setting" },
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
      <header class="titlebar">
        <div class="titlebar-drag" data-tauri-drag-region>
          <span class="titlebar-title" data-tauri-drag-region>ClashEdge</span>
        </div>
        <div class="titlebar-controls">
          <button
            type="button"
            class="tb-btn"
            :title="$t('titlebar.minimize')"
            :aria-label="$t('titlebar.minimize')"
            @click="onMinimize"
          >
            <el-icon :size="14"><Minus /></el-icon>
          </button>
          <button
            type="button"
            class="tb-btn"
            :title="isMaximized ? $t('titlebar.restore') : $t('titlebar.maximize')"
            :aria-label="isMaximized ? $t('titlebar.restore') : $t('titlebar.maximize')"
            @click="onToggleMaximize"
          >
            <el-icon :size="13">
              <FullScreen v-if="!isMaximized" />
              <CopyDocument v-else />
            </el-icon>
          </button>
          <button
            type="button"
            class="tb-btn tb-close"
            :title="$t('titlebar.close')"
            :aria-label="$t('titlebar.close')"
            @click="onClose"
          >
            <el-icon :size="14"><Close /></el-icon>
          </button>
        </div>
      </header>

      <el-container class="app-shell">
        <el-aside :width="isNarrow ? '64px' : '216px'" class="app-aside" :class="{ narrow: isNarrow }">
          <el-menu :default-active="route.path" router class="app-menu">
            <el-menu-item
              v-for="item in menuItems"
              :key="item.path"
              :index="item.path"
              :title="$t(item.key)"
            >
              <el-icon><component :is="item.icon" /></el-icon>
              <span class="menu-label">{{ $t(item.key) }}</span>
            </el-menu-item>
          </el-menu>
          <div v-if="appStore.version && !isNarrow" class="app-footer">
            v{{ appStore.version }}
          </div>
        </el-aside>
        <el-main class="app-main">
          <el-alert
            v-if="coreError"
            :title="coreError"
            type="error"
            show-icon
            :closable="false"
            class="core-error-banner"
          />
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
  background-color: var(--bg-app);
}

.titlebar {
  flex: none;
  height: 36px;
  display: flex;
  align-items: stretch;
  background-color: var(--bg-surface);
  border-bottom: 1px solid var(--border-subtle);
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
  color: var(--text-secondary);
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
  color: var(--text-secondary);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition:
    background-color 0.15s ease,
    color 0.15s ease;
}

.tb-btn:hover {
  background-color: var(--interactive-hover);
  color: var(--text-primary);
}

.tb-close:hover {
  background-color: var(--error);
  color: var(--on-error);
}

/* 键盘可达性：标题栏按钮聚焦时显示清晰焦点环。 */
.tb-btn:focus-visible {
  outline: 2px solid var(--accent);
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

.core-error-banner {
  margin-bottom: 12px;
}
</style>
