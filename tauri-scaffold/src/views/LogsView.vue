<!-- src/views/LogsView.vue - 日志：通过 Mihomo 外部控制器 /logs 实时流展示
     后端 core::logs 连接控制器 SSE 长连接，逐行转发为 log-line 事件；
     本页挂载时启动流、卸载时停止，保证不残留后台连接。 -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { logsApi } from "@/api/logs";
import { useCoreStore } from "@/stores/core";

interface LogEntry {
  id: number;
  level: string;
  message: string;
}

const core = useCoreStore();

const entries = ref<LogEntry[]>([]);
const connected = ref(false);
const connecting = ref(false);
const errorMsg = ref("");
const MAX_LINES = 500;
let idSeq = 0;
let everConnected = false;
let unlisteners: UnlistenFn[] = [];

const listEl = ref<HTMLElement | null>(null);

// 「自动滚动到底」开关：默认开。强制滚动仅在「开关开 && 视口处于底部」时执行，
// 用户向上滚动离开底部即自然暂停，重新触底或重新打开开关后恢复。
const follow = ref(true);

/** 触底判定：scrollTop + clientHeight >= scrollHeight - 8。 */
function isAtBottom(el: HTMLElement): boolean {
  return el.scrollTop + el.clientHeight >= el.scrollHeight - 8;
}

function scrollToBottom() {
  const el = listEl.value;
  if (el) el.scrollTop = el.scrollHeight;
}

const statusText = computed(() => {
  if (connected.value) return "logs.connected";
  if (errorMsg.value) return "logs.disconnected";
  return "logs.waiting";
});

function push(level: string, message: string) {
  entries.value.push({ id: idSeq++, level, message });
  if (entries.value.length > MAX_LINES) {
    entries.value.splice(0, entries.value.length - MAX_LINES);
  }
  // 自动跟随到底部（下一帧，等 DOM 更新）：仅在开关开启且当前处于底部时执行。
  requestAnimationFrame(() => {
    const el = listEl.value;
    if (el && follow.value && isAtBottom(el)) el.scrollTop = el.scrollHeight;
  });
}

/** 开关切换：打开时立即滚到底部，恢复跟随。 */
watch(follow, (v) => {
  if (v) scrollToBottom();
});

function clear() {
  entries.value = [];
}

async function connect() {
  // 幂等守卫：已连接/连接中直接返回，防止重复启动后端 SSE 导致日志翻倍。
  if (connected.value || connecting.value) return;
  connecting.value = true;
  errorMsg.value = "";
  try {
    // 曾连接过则先停旧流，避免后端残留旧 SSE 连接。
    if (everConnected) await logsApi.stop();
    await logsApi.start();
  } catch (e) {
    errorMsg.value = String(e);
  } finally {
    connecting.value = false;
  }
}

const levelClass = (level: string): string =>
  level === "error"
    ? "lv-error"
    : level === "warning"
      ? "lv-warning"
      : level === "debug"
        ? "lv-debug"
        : "lv-info";

onMounted(async () => {
  unlisteners.push(
    await listen<{ level: string; message: string }>("log-line", (ev) => {
      connected.value = true;
      everConnected = true;
      errorMsg.value = "";
      push(ev.payload.level, ev.payload.message);
    })
  );
  unlisteners.push(
    await listen("log-connected", () => {
      connected.value = true;
      everConnected = true;
      errorMsg.value = "";
    })
  );
  unlisteners.push(
    await listen<{ error: string }>("log-error", (ev) => {
      connected.value = false;
      errorMsg.value = ev.payload.error;
    })
  );
  await connect();
});

onUnmounted(() => {
  unlisteners.forEach((u) => u());
  void logsApi.stop().catch(() => {});
});
</script>

<template>
  <div class="page">
    <h2 class="page-title">{{ $t("logs.title") }}</h2>

    <div class="log-toolbar">
      <span class="status-pill" :class="{ running: connected }">
        <span class="status-dot" aria-hidden="true"></span>
        {{ $t(statusText) }}
      </span>
      <el-button :disabled="!core.status.running" :loading="connecting" @click="connect">
        {{ $t("logs.reconnect") }}
      </el-button>
      <el-button @click="clear">{{ $t("logs.clear") }}</el-button>
      <span class="log-follow">
        <span class="log-follow-label">{{ $t("logs.auto_scroll") }}</span>
        <el-switch v-model="follow" size="small" />
      </span>
    </div>

    <el-empty
      v-if="!core.status.running"
      class="log-empty"
      :description="$t('logs.not_running')"
    />

    <div v-else ref="listEl" class="log-scroll">
      <div v-if="entries.length === 0" class="log-placeholder">
        {{ $t(errorMsg ? "logs.disconnected" : "logs.waiting") }}
      </div>
      <div v-for="e in entries" :key="e.id" class="log-line">
        <span class="log-level" :class="levelClass(e.level)">{{ e.level.toUpperCase() }}</span>
        <span class="log-msg">{{ e.message }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 状态指示由全局 .status-pill/.status-dot/.running 提供（styles.css），
 * 与概览页共用同一套语义色。 */

.log-toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-bottom: var(--space-3);
}

.log-follow {
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.log-follow-label {
  font-size: 12px;
  color: var(--text-tertiary);
}

.log-scroll {
  height: calc(100vh - 160px);
  overflow-y: auto;
  background: var(--bg-raised);
  border: 1px solid var(--card-border);
  border-radius: var(--r-md);
  padding: 10px 12px;
  font-family: "Consolas", "Menlo", monospace;
  font-size: 12px;
}

.log-line {
  display: flex;
  align-items: baseline;
  gap: 10px;
  padding: 3px 6px;
  border-radius: var(--el-border-radius-small);
  color: var(--text-secondary);
  transition: background-color var(--dur-fast) ease;
}

.log-line:hover {
  background: var(--interactive-hover);
}

/* 级别文字：固定宽度右对齐，语义色统一（error/warning 醒目，debug/info 弱化），
 * 不给整行铺背景，保持日志工具化可读。 */
.log-level {
  flex: none;
  width: 56px;
  text-align: left;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.3px;
  font-variant-numeric: tabular-nums;
  user-select: none;
}

.log-level.lv-error {
  color: var(--error);
}

.log-level.lv-warning {
  color: var(--approval);
}

.log-level.lv-debug {
  color: var(--text-tertiary);
}

.log-level.lv-info {
  color: var(--accent);
}

.log-msg {
  word-break: break-all;
  white-space: pre-wrap;
}

.log-placeholder {
  color: var(--text-tertiary);
  text-align: center;
  padding: 32px 0;
}
</style>
