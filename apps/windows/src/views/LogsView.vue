<!-- src/views/LogsView.vue - 日志：通过 Mihomo 外部控制器 /logs 实时流展示
     后端 core::logs 连接控制器 SSE 长连接，逐行转发为 log-line 事件；
     本页挂载时启动流、卸载时停止，保证不残留后台连接。
     - 级别筛选芯片（带计数；形状：DEBUG 无 / INFO 圆 / WARN 菱形 / ERROR 横杠）+ 正则搜索
     - 暂停 / 清空…（确认）；向上滚动即暂停跟随，出现「回到最新」
     - 导出需要「保存文件」能力（尚无对应后端命令），本页暂不含 -->
<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowDown, Pause, Play, Search } from "lucide-vue-next";
import StatusPill from "@/components/StatusPill.vue";
import { useConfirm } from "@/composables/useConfirm";
import { useLogStream } from "@/composables/useLogStream";
import { useCoreStore } from "@/stores/core";

const { t } = useI18n();
const core = useCoreStore();
const confirm = useConfirm();
const {
  entries,
  connected,
  connecting,
  errorMsg,
  listEl,
  paused,
  atBottom,
  statusKey,
  connect,
  clear,
  pause,
  resume,
  onScroll,
  jumpToLatest,
} = useLogStream();
// `listEl` 只通过模板 ref="listEl" 绑定（vue-tsc 看不到这种使用）。
void listEl;

type LevelId = "debug" | "info" | "warning" | "error";
const LEVELS: { id: LevelId; label: string }[] = [
  { id: "debug", label: "DEBUG" },
  { id: "info", label: "INFO" },
  { id: "warning", label: "WARN" },
  { id: "error", label: "ERROR" },
];
const levelOf = (raw: string): LevelId =>
  raw === "error" || raw === "warning" || raw === "debug" ? raw : "info";

const level = ref<LevelId | "all">("all");
const query = ref("");

const counts = computed(() => {
  const c: Record<LevelId, number> = { debug: 0, info: 0, warning: 0, error: 0 };
  for (const e of entries.value) c[levelOf(e.level)]++;
  return c;
});

/** 支持正则；输入了不合法的正则时退回为普通子串匹配，而不是抛错。 */
const matcher = computed<(s: string) => boolean>(() => {
  const q = query.value.trim();
  if (!q) return () => true;
  try {
    const re = new RegExp(q, "i");
    return (s) => re.test(s);
  } catch {
    const lower = q.toLowerCase();
    return (s) => s.toLowerCase().includes(lower);
  }
});

const visible = computed(() =>
  entries.value.filter(
    (e) => (level.value === "all" || levelOf(e.level) === level.value) && matcher.value(e.message),
  ),
);

async function onClear() {
  const ok = await confirm(t("logs.clear_body"), {
    title: t("logs.clear_title"),
    confirmText: t("logs.clear_confirm"),
    cancelText: t("common.cancel"),
    danger: true,
  });
  if (ok) clear();
}
</script>

<template>
  <div class="page">
    <div class="toolbar">
      <h2 class="page-title">{{ $t("logs.title") }}</h2>
      <StatusPill :active="connected" :label="$t(statusKey)" />
      <span class="toolbar-spacer"></span>
      <label class="search-box">
        <Search :size="16" :stroke-width="1.75" aria-hidden="true" />
        <input
          v-model="query"
          type="search"
          :placeholder="$t('logs.search_placeholder')"
          :aria-label="$t('logs.search_placeholder')"
        />
      </label>
      <el-button :disabled="!core.status.running" :loading="connecting" @click="connect">
        {{ $t("logs.reconnect") }}
      </el-button>
      <el-button class="pause-btn" :aria-pressed="paused" @click="paused ? resume() : pause()">
        <component :is="paused ? Play : Pause" :size="16" :stroke-width="1.75" aria-hidden="true" />
        {{ paused ? $t("logs.resume") : $t("logs.pause") }}
      </el-button>
      <el-button text class="clear-btn" :disabled="entries.length === 0" @click="onClear">
        {{ $t("logs.clear") }}…
      </el-button>
    </div>

    <div class="chips" role="group" :aria-label="$t('logs.level_aria')">
      <button
        type="button"
        class="chip"
        :class="{ 'is-on': level === 'all' }"
        :aria-pressed="level === 'all'"
        @click="level = 'all'"
      >
        {{ $t("logs.level_all") }} <span class="chip-n">{{ entries.length }}</span>
      </button>
      <button
        v-for="l in LEVELS"
        :key="l.id"
        type="button"
        class="chip"
        :class="{ 'is-on': level === l.id }"
        :aria-pressed="level === l.id"
        @click="level = l.id"
      >
        <i class="mark" :class="`mark-${l.id}`" aria-hidden="true"></i>{{ l.label }}
        <span class="chip-n">{{ counts[l.id] }}</span>
      </button>
    </div>

    <el-empty
      v-if="!core.status.running"
      class="log-empty"
      :description="$t('logs.not_running')"
    />

    <div v-else class="log-wrap ce-card">
      <div ref="listEl" class="log-scroll" role="log" aria-live="off" tabindex="0" :aria-label="$t('logs.title')" @scroll="onScroll">
        <div v-if="entries.length === 0" class="log-placeholder">
          {{ $t(errorMsg ? "logs.disconnected" : "logs.waiting") }}
        </div>
        <div v-else-if="visible.length === 0" class="log-placeholder">{{ $t("logs.no_match") }}</div>
        <div v-for="e in visible" :key="e.id" class="log-line">
          <span class="log-time">{{ e.time }}</span>
          <span class="log-level" :class="`lv-${levelOf(e.level)}`">
            <i class="mark" :class="`mark-${levelOf(e.level)}`" aria-hidden="true"></i>{{ LEVELS.find((l) => l.id === levelOf(e.level))?.label }}
          </span>
          <span class="log-msg">{{ e.message }}</span>
        </div>
      </div>
      <button v-if="!atBottom" type="button" class="jump" @click="jumpToLatest">
        {{ $t("logs.jump_latest") }}<ArrowDown :size="16" :stroke-width="1.75" aria-hidden="true" />
      </button>
    </div>
  </div>
</template>

<style scoped>
/* 状态指示样式见 components/StatusPill.vue，
 * 与首页共用同一套语义色。 */

.toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ce-space-3);
  margin-bottom: var(--ce-space-3);
}

.toolbar .page-title {
  margin: 0;
}

.toolbar-spacer {
  flex: 1;
}

.search-box {
  display: flex;
  align-items: center;
  gap: var(--ce-space-2);
  width: 260px;
  height: var(--ce-control-h);
  padding: 0 var(--ce-space-3);
  border: 1px solid transparent;
  border-radius: var(--ce-radius-md);
  background: var(--ce-fill-soft);
  color: var(--ce-text-tertiary);
}

.search-box:focus-within {
  border-color: var(--ce-accent-bg);
  background: var(--ce-bg-surface);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--ce-accent-bg) 18%, transparent);
}

.search-box input {
  flex: 1;
  min-width: 0;
  height: 100%;
  border: 0;
  outline: none;
  background: transparent;
  color: var(--ce-text-primary);
  font: var(--ce-type-body);
}

.clear-btn {
  color: var(--ce-danger-fg);
}

.clear-btn:hover:not(.is-disabled) {
  color: var(--ce-danger-fg);
  background: var(--ce-danger-soft);
}

.chips {
  display: flex;
  flex-wrap: wrap;
  gap: var(--ce-space-2);
  margin-bottom: var(--ce-space-4);
}

.chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: var(--ce-control-h-sm);
  padding: 0 var(--ce-space-3);
  border: 1px solid var(--ce-border);
  border-radius: var(--ce-radius-full);
  background: var(--ce-bg-surface);
  color: var(--ce-text-secondary);
  font: var(--ce-type-callout);
  cursor: pointer;
}

.chip:hover {
  background: var(--ce-fill-hover);
}

.chip.is-on {
  border-color: transparent;
  background: var(--ce-accent-soft);
  color: var(--ce-accent-fg);
  font-weight: 500;
}

.chip:focus-visible,
.jump:focus-visible {
  outline: 2px solid var(--ce-accent-bg);
  outline-offset: 2px;
}

.chip-n {
  color: var(--ce-text-tertiary);
  font: var(--ce-type-caption);
  font-variant-numeric: tabular-nums;
}

.chip.is-on .chip-n {
  color: var(--ce-accent-fg);
}

/* 级别形状：DEBUG 无形状 / INFO 圆 / WARN 菱形 / ERROR 横杠。 */
.mark {
  display: inline-block;
  flex: none;
}

.mark-debug {
  display: none;
}

.mark-info {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--ce-accent-bg);
}

.mark-warning {
  width: 7px;
  height: 7px;
  transform: rotate(45deg);
  background: var(--ce-warning-solid);
}

.mark-error {
  width: 9px;
  height: 3px;
  border-radius: var(--ce-radius-full);
  background: var(--ce-danger-solid);
}

.log-wrap {
  position: relative;
  padding: 0;
  overflow: hidden;
}

.log-scroll {
  height: calc(100vh - 220px);
  padding: var(--ce-space-2) var(--ce-space-3);
  overflow-y: auto;
  font: var(--ce-type-mono);
}

.log-line {
  display: grid;
  grid-template-columns: 76px 72px minmax(0, 1fr);
  align-items: baseline;
  gap: var(--ce-space-3);
  padding: 2px var(--ce-space-2);
  border-radius: var(--ce-radius-xs);
  color: var(--ce-text-secondary);
}

.log-line:hover {
  background: var(--ce-fill-hover);
}

.log-time {
  color: var(--ce-text-tertiary);
  font-variant-numeric: tabular-nums;
}

.log-level {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font: var(--ce-type-caption);
  font-weight: 600;
  user-select: none;
}

.log-level.lv-error {
  color: var(--ce-danger-fg);
}

.log-level.lv-warning {
  color: var(--ce-warning-fg);
}

.log-level.lv-debug {
  color: var(--ce-text-tertiary);
}

.log-level.lv-info {
  color: var(--ce-accent-fg);
}

.log-msg {
  white-space: pre-wrap;
  word-break: break-all;
  color: var(--ce-text-primary);
}

.log-placeholder {
  padding: var(--ce-space-8) 0;
  text-align: center;
  color: var(--ce-text-tertiary);
  font: var(--ce-type-callout);
}

.jump {
  position: absolute;
  left: 50%;
  bottom: var(--ce-space-4);
  display: inline-flex;
  align-items: center;
  gap: var(--ce-space-1);
  height: var(--ce-control-h-sm);
  padding: 0 var(--ce-space-3);
  border: 0;
  border-radius: var(--ce-radius-full);
  background: var(--ce-bg-elevated);
  box-shadow: var(--ce-shadow-e2);
  color: var(--ce-text-primary);
  font: var(--ce-type-callout);
  font-weight: 500;
  transform: translateX(-50%);
  cursor: pointer;
}

.jump:hover {
  background: var(--ce-fill-hover);
}
</style>
