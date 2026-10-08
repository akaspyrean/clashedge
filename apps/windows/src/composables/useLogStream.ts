// src/composables/useLogStream.ts - 日志页的数据层：事件监听、环形缓冲、自动跟随、可见性暂停。
//
// 后端 core::logs 连接控制器 SSE，逐行转发 `log-line` 事件。本 composable 必须在组件 setup
// 内调用：挂载时启动流、卸载时停止，保证不残留后台连接。

import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { logsApi } from "@/api/logs";

export interface LogEntry {
  id: number;
  level: string;
  message: string;
}

export const MAX_LOG_LINES = 500;

export function levelClass(level: string): string {
  return level === "error"
    ? "lv-error"
    : level === "warning"
      ? "lv-warning"
      : level === "debug"
        ? "lv-debug"
        : "lv-info";
}

/** 触底判定：scrollTop + clientHeight >= scrollHeight - 8。 */
function isAtBottom(el: HTMLElement): boolean {
  return el.scrollTop + el.clientHeight >= el.scrollHeight - 8;
}

export function useLogStream() {
  const entries = ref<LogEntry[]>([]);
  const connected = ref(false);
  const connecting = ref(false);
  const errorMsg = ref("");
  const listEl = ref<HTMLElement | null>(null);
  // 默认跟随；用户向上滚动离开底部即自然暂停，重新触底或重开开关后恢复。
  const follow = ref(true);

  let idSeq = 0;
  let everConnected = false;
  let pausedByHidden = false;
  let unlisteners: UnlistenFn[] = [];

  const statusKey = computed(() =>
    connected.value ? "logs.connected" : errorMsg.value ? "logs.disconnected" : "logs.waiting",
  );

  function scrollToBottom() {
    const el = listEl.value;
    if (el) el.scrollTop = el.scrollHeight;
  }

  function push(level: string, message: string) {
    entries.value.push({ id: idSeq++, level, message });
    if (entries.value.length > MAX_LOG_LINES) {
      entries.value.splice(0, entries.value.length - MAX_LOG_LINES);
    }
    requestAnimationFrame(() => {
      const el = listEl.value;
      if (el && follow.value && isAtBottom(el)) el.scrollTop = el.scrollHeight;
    });
  }

  watch(follow, (v) => {
    if (v) scrollToBottom();
  });

  function clear() {
    entries.value = [];
  }

  async function connect() {
    // 幂等守卫：已连接 / 连接中直接返回，防止重复启动后端 SSE 导致日志翻倍。
    if (connected.value || connecting.value) return;
    connecting.value = true;
    errorMsg.value = "";
    try {
      if (everConnected) await logsApi.stop();
      await logsApi.start();
    } catch (e) {
      errorMsg.value = String(e);
    } finally {
      connecting.value = false;
    }
  }

  /** 窗口隐藏到托盘时暂停后端 SSE，重新可见时恢复（省 CPU / IPC 事件）。 */
  function onVisibilityChange() {
    if (document.hidden) {
      if (connected.value || connecting.value) {
        pausedByHidden = true;
        void logsApi.stop().catch(() => {});
        connected.value = false;
      }
    } else if (pausedByHidden) {
      pausedByHidden = false;
      void connect();
    }
  }

  onMounted(async () => {
    document.addEventListener("visibilitychange", onVisibilityChange);
    unlisteners.push(
      await listen<{ level: string; message: string }>("log-line", (ev) => {
        connected.value = true;
        everConnected = true;
        errorMsg.value = "";
        push(ev.payload.level, ev.payload.message);
      }),
      await listen("log-connected", () => {
        connected.value = true;
        everConnected = true;
        errorMsg.value = "";
      }),
      await listen<{ error: string }>("log-error", (ev) => {
        connected.value = false;
        errorMsg.value = ev.payload.error;
      }),
    );
    await connect();
  });

  onUnmounted(() => {
    document.removeEventListener("visibilitychange", onVisibilityChange);
    unlisteners.forEach((u) => u());
    unlisteners = [];
    void logsApi.stop().catch(() => {});
  });

  return { entries, connected, connecting, errorMsg, listEl, follow, statusKey, connect, clear };
}
