// src/composables/useLogStream.ts - 日志页的数据层：事件监听、环形缓冲、自动跟随、可见性暂停。
//
// 后端 core::logs 连接控制器 SSE，逐行转发 `log-line` 事件。本 composable 必须在组件 setup
// 内调用：挂载时启动流、卸载时停止，保证不残留后台连接。

import { computed, markRaw, onMounted, onUnmounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { logsApi } from "@/api/logs";
import { maskUrlsInText } from "@/utils/format";

export interface LogEntry {
  id: number;
  /** 前端收到该行的本地时间 HH:MM:SS（SSE 事件不带时间戳）。 */
  time: string;
  level: string;
  message: string;
}

function clock(d = new Date()): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
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
  // 使用 ref 保证响应式，对每条 entry 用 markRaw 避免深层 Proxy 开销。
  const entries = ref<LogEntry[]>([]);
  const connected = ref(false);
  const connecting = ref(false);
  const errorMsg = ref("");
  const listEl = ref<HTMLElement | null>(null);
  // 默认跟随；用户向上滚动离开底部即自然暂停，重新触底或重开开关后恢复。
  const follow = ref(true);
  // 暂停：新日志先进 pending，恢复时一并并入，暂停期间列表不再跳动，方便阅读。
  const paused = ref(false);
  const pending: LogEntry[] = [];
  /** 视图是否停在底部；离开底部时显示「回到最新」。 */
  const atBottom = ref(true);

  // 增量维护各级别计数，避免每次追加日志全量遍历 500 条。
  const counts = ref<Record<string, number>>({ debug: 0, info: 0, warning: 0, error: 0 });

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

  function append(list: LogEntry[]) {
    const arr = entries.value;
    const removed = arr.length + list.length > MAX_LOG_LINES
      ? arr.splice(0, arr.length + list.length - MAX_LOG_LINES)
      : [];
    // 减去被移除条目的计数
    for (const e of removed) {
      const lvl = e.level.toLowerCase();
      if (counts.value[lvl] !== undefined) counts.value[lvl] = Math.max(0, counts.value[lvl] - 1);
    }
    // markRaw 单条 entry 避免深层 Proxy，arr.push 仍能触发 ref 的响应式
    for (const e of list) markRaw(e);
    arr.push(...list);
    // 加上新条目的计数
    for (const e of list) {
      const lvl = e.level.toLowerCase();
      if (counts.value[lvl] !== undefined) counts.value[lvl] = (counts.value[lvl] ?? 0) + 1;
    }
    requestAnimationFrame(() => {
      const el = listEl.value;
      if (el && follow.value && atBottom.value) el.scrollTop = el.scrollHeight;
    });
  }

  function push(level: string, message: string) {
    // 入队前打码日志行内嵌的订阅/下载 URL（mihomo 在 info 级会打印 provider
    // 更新地址，含 token；用户复制日志分享时不应带出）。
    const entry: LogEntry = { id: idSeq++, time: clock(), level, message: maskUrlsInText(message) };
    if (paused.value) {
      pending.push(entry);
      if (pending.length > MAX_LOG_LINES) pending.splice(0, pending.length - MAX_LOG_LINES);
      return;
    }
    append([entry]);
  }

  function pause() {
    paused.value = true;
  }

  function resume() {
    paused.value = false;
    if (pending.length) append(pending.splice(0, pending.length));
  }

  /** 视图的 scroll 事件调用：更新「是否在底部」。向上滚动即自然暂停跟随。 */
  function onScroll() {
    const el = listEl.value;
    if (el) atBottom.value = isAtBottom(el);
  }

  function jumpToLatest() {
    atBottom.value = true;
    scrollToBottom();
  }

  watch(follow, (v) => {
    if (v) scrollToBottom();
  });

  function clear() {
    entries.value = [];
    pending.length = 0;
    counts.value = { debug: 0, info: 0, warning: 0, error: 0 };
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
        connecting.value = false; // 关键：重置连接中状态，允许可见时重新连接
      }
    } else if (pausedByHidden) {
      pausedByHidden = false;
      void connect();
    }
  }

  onMounted(async () => {
    document.addEventListener("visibilitychange", onVisibilityChange);
    unlisteners.push(
      await listen<{ level?: string; message?: string; batch?: Array<{ level: string; message: string } } }>(
        "log-line",
        (ev) => {
          connected.value = true;
          everConnected = true;
          errorMsg.value = "";
          if (ev.payload.batch) {
            // 批量推送：一次性并入多条日志，减少响应式触发次数
            const entries = ev.payload.batch.map((e) => ({
              id: idSeq++,
              time: clock(),
              level: e.level,
              message: maskUrlsInText(e.message),
            }));
            append(entries);
          } else {
            push(ev.payload.level ?? "info", ev.payload.message ?? "");
          }
        },
      ),
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

  return {
    entries,
    connected,
    connecting,
    errorMsg,
    listEl,
    follow,
    paused,
    atBottom,
    statusKey,
    counts,
    connect,
    clear,
    pause,
    resume,
    onScroll,
    jumpToLatest,
  };
}
