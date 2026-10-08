<!-- src/views/LogsView.vue - 日志：通过 Mihomo 外部控制器 /logs 实时流展示
     后端 core::logs 连接控制器 SSE 长连接，逐行转发为 log-line 事件；
     本页挂载时启动流、卸载时停止，保证不残留后台连接。 -->
<script setup lang="ts">
import StatusPill from "@/components/StatusPill.vue";
import { levelClass, useLogStream } from "@/composables/useLogStream";
import { useCoreStore } from "@/stores/core";

const core = useCoreStore();
const { entries, connected, connecting, errorMsg, listEl, follow, statusKey, connect, clear } =
  useLogStream();
// `listEl` 只通过模板 ref="listEl" 绑定（vue-tsc 看不到这种使用）。
void listEl;
</script>

<template>
  <div class="page">
    <h2 class="page-title">{{ $t("logs.title") }}</h2>

    <div class="log-toolbar">
      <StatusPill :active="connected" :label="$t(statusKey)" />
      <el-button :disabled="!core.status.running" :loading="connecting" @click="connect">
        {{ $t("logs.reconnect") }}
      </el-button>
      <el-button @click="clear">{{ $t("logs.clear") }}</el-button>
      <span class="log-follow">
        <span class="log-follow-label">{{ $t("logs.auto_scroll") }}</span>
        <el-switch v-model="follow" size="small" :aria-label="$t('logs.auto_scroll')" />
      </span>
    </div>

    <el-empty
      v-if="!core.status.running"
      class="log-empty"
      :description="$t('logs.not_running')"
    />

    <div v-else ref="listEl" class="log-scroll" role="log" aria-live="off" tabindex="0" :aria-label="$t('logs.title')">
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
  font-size: 12px;
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
