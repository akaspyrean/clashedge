// src/api/logs.ts
// 日志流：后端连接 mihomo 外部控制器 /logs（SSE），通过 log-line 事件推给前端。
// 此处只负责启动/停止后端任务；日志内容走事件监听（见 views/LogsView.vue）。

import { invoke } from "@tauri-apps/api/core";

export const logsApi = {
  start: () => invoke<void>("start_log_stream"),
  stop: () => invoke<void>("stop_log_stream"),
};
