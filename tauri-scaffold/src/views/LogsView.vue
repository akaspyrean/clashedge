<!-- src/views/LogsView.vue - 日志：通过 Mihomo 外部控制器 /logs 实时流展示
     后端 core::logs 连接控制器 SSE 长连接，逐行转发为 log-line 事件；
     本页挂载时启动流、卸载时停止，保证不残留后台连接。 -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
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
const errorMsg = ref("");
const MAX_LINES = 500;
let idSeq = 0;
let unlisteners: UnlistenFn[] = [];

const listEl = ref<HTMLElement | null>(null);

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
  // 自动跟随到底部（下一帧，等 DOM 更新）
  requestAnimationFrame(() => {
    if (listEl.value) listEl.value.scrollTop = listEl.value.scrollHeight;
  });
}

function clear() {
  entries.value = [];
}

async function connect() {
  errorMsg.value = "";
  connected.value = false;
  try {
    await logsApi.start();
  } catch (e) {
    errorMsg.value = String(e);
  }
}

const levelType = (level: string) =>
  level === "error"
    ? "danger"
    : level === "warning"
      ? "warning"
      : level === "debug"
        ? "info"
        : "success";

onMounted(async () => {
  unlisteners.push(
    await listen<{ level: string; message: string }>("log-line", (ev) => {
      connected.value = true;
      errorMsg.value = "";
      push(ev.payload.level, ev.payload.message);
    })
  );
  unlisteners.push(
    await listen("log-connected", () => {
      connected.value = true;
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
      <el-tag :type="connected ? 'success' : 'info'" size="small">
        {{ $t(statusText) }}
      </el-tag>
      <el-button size="small" :disabled="!core.status.running" @click="connect">
        {{ $t("logs.reconnect") }}
      </el-button>
      <el-button size="small" @click="clear">{{ $t("logs.clear") }}</el-button>
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
        <el-tag :type="levelType(e.level)" size="small" effect="plain">
          {{ e.level.toUpperCase() }}
        </el-tag>
        <span class="log-msg">{{ e.message }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.log-toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
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
  border-radius: 4px;
  color: var(--text-secondary);
  transition: background-color 0.15s ease;
}

.log-line:hover {
  background: var(--interactive-hover);
}

.log-line .el-tag {
  flex: none;
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

.log-empty {
  padding: 40px 0;
}
</style>
