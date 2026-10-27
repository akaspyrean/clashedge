<!-- src/views/ConnectionsView.vue - 连接：顶部实时速率 + 60 秒流量图，下方活动连接表。
     - 顶部卡片：左「↓下载 / ↑上传」速率，右一行「本次会话 ↓x · ↑y · N 个活动连接」+ CeTrafficChart
     - 连接表：主机 · 规则 → 链路 · 类型 · ↓ · ↑ · 时长；筛选框；「全部断开…」走 useConfirm
     - 轮询沿用 usePolling + store 的自适应间隔；后端已裁剪到 500 条，仍保留截断提示
     - 暂无「已关闭」数据来源，故只做「活动」；单条断开 / 进程名后端未提供，本页不含 -->
<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Search } from "lucide-vue-next";
import CeTrafficChart from "@/components/ui/CeTrafficChart.vue";
import { useAction } from "@/composables/useAction";
import { useConfirm } from "@/composables/useConfirm";
import { usePolling } from "@/composables/usePolling";
import { pollIntervalFor, useConnectionsStore } from "@/stores/connections";
import { formatBytes, splitRate } from "@/utils/format";

const MAX_DISPLAY = 500;

const { t } = useI18n();
const store = useConnectionsStore();
const closing = useAction();
const confirm = useConfirm();

const chartRef = ref<InstanceType<typeof CeTrafficChart> | null>(null);
const query = ref("");
const connectionCount = computed(() => store.count);
const downRate = computed(() => splitRate(store.rateDown[store.rateDown.length - 1] ?? 0));
const upRate = computed(() => splitRate(store.rateUp[store.rateUp.length - 1] ?? 0));

function toggleChartTable() {
  chartRef.value?.toggleTable();
}

const rows = computed(() => {
  const q = query.value.trim().toLowerCase();
  const all = store.connections.map((c) => ({
    ...c,
    // mihomo 的 chains 以最终出口节点开头，反转后按「入口组 → … → 节点」阅读。
    chain: [...c.chains].reverse().join(" → "),
    kind: c.type ? `${c.network} · ${c.type}` : c.network,
  }));
  if (!q) return all;
  return all.filter((c) => [c.host, c.rule, c.chain, c.kind].some((s) => s.toLowerCase().includes(q)));
});

/** start 为 Unix 毫秒时间戳 → 显示连接已持续的时长（mm:ss / hh:mm:ss）。 */
function formatStart(start: number): string {
  if (!Number.isFinite(start) || start <= 0) return "—";
  const elapsed = Math.max(0, Math.floor((Date.now() - start) / 1000));
  const h = Math.floor(elapsed / 3600);
  const m = Math.floor((elapsed % 3600) / 60);
  const s = elapsed % 60;
  const mm = m.toString().padStart(2, "0");
  const ss = s.toString().padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

const { tick } = usePolling(
  () => store.refresh(),
  () => pollIntervalFor(store.count),
);

async function onCloseAll() {
  const ok = await confirm(t("connections.close_all_body", { n: connectionCount.value }), {
    title: t("connections.close_all_title"),
    confirmText: t("connections.close_all_confirm"),
    cancelText: t("common.cancel"),
    danger: true,
  });
  if (!ok) return;
  const r = await closing.run(() => store.closeAll(), { silent: true });
  if (r.ok) void tick();
}
</script>

<template>
  <div class="page connections-page">
    <section class="ce-card overview" :aria-label="$t('connections.overview')">
      <div class="stats">
        <div class="stat">
          <span class="stat-label" style="color: var(--ce-chart-down)">↓ {{ $t("connections.download_rate") }}</span>
          <span class="stat-figures">
            <span class="stat-value">{{ downRate[0] }}</span>
            <span class="stat-unit">{{ downRate[1] }}</span>
          </span>
        </div>
        <div class="stat">
          <span class="stat-label" style="color: var(--ce-chart-up)">↑ {{ $t("connections.upload_rate") }}</span>
          <span class="stat-figures">
            <span class="stat-value">{{ upRate[0] }}</span>
            <span class="stat-unit">{{ upRate[1] }}</span>
          </span>
        </div>
        <div class="stat session">
          <button
            type="button"
            class="stat-label session-summary"
            :aria-expanded="chartRef?.showTable ? 'true' : 'false'"
            @click="toggleChartTable"
          >
            {{
              $t("connections.session_summary", {
                down: formatBytes(store.downloadTotal),
                up: formatBytes(store.uploadTotal),
                n: connectionCount,
              })
            }}
          </button>
        </div>
      </div>
      <CeTrafficChart ref="chartRef" :down="store.rateDown" :up="store.rateUp" :show-legend="false" :show-table-toggle="false" />
    </section>

    <div class="list-head">
      <label class="search-box">
        <Search :size="16" :stroke-width="1.75" aria-hidden="true" />
        <input
          v-model="query"
          type="search"
          :placeholder="$t('connections.filter_placeholder')"
          :aria-label="$t('connections.filter_placeholder')"
        />
      </label>
      <el-button
        text
        class="close-all"
        :disabled="connectionCount === 0"
        :loading="closing.busy.value"
        @click="onCloseAll"
      >
        {{ $t("connections.close_all") }}
      </el-button>
    </div>

    <section v-if="store.connections.length > 0" class="ce-card connections-table" :aria-label="$t('connections.title')">
      <div class="grid head">
        <span>{{ $t("connections.host") }}</span>
        <span>{{ $t("connections.rule_chain") }}</span>
        <span>{{ $t("connections.kind") }}</span>
        <span class="num" style="color: var(--ce-chart-down)">↓</span>
        <span class="num" style="color: var(--ce-chart-up)">↑</span>
        <span class="num">{{ $t("connections.time") }}</span>
      </div>
      <ul class="conn-list">
        <li v-for="c in rows" :key="c.id" class="grid">
          <span class="host" :title="c.host">{{ c.host || "—" }}</span>
          <span class="route">
            <span class="rule" :title="c.rule">{{ c.rule }}</span>
            <span v-if="c.chain" class="chain" :title="c.chain">{{ c.chain }}</span>
          </span>
          <span><span class="kind-tag">{{ c.kind }}</span></span>
          <span class="num mono">{{ formatBytes(c.download) }}</span>
          <span class="num mono">{{ formatBytes(c.upload) }}</span>
          <span class="num mono dim">{{ formatStart(c.start) }}</span>
        </li>
      </ul>
      <p v-if="rows.length === 0" class="no-match">{{ $t("connections.no_match") }}</p>
    </section>

    <el-empty v-else :description="$t('connections.empty')" />

    <div v-if="connectionCount > MAX_DISPLAY" class="truncated-notice" role="status">
      {{ $t("connections.truncated_notice", { max: MAX_DISPLAY, count: connectionCount }) }}
    </div>
  </div>
</template>

<style scoped>
.overview {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-4);
  margin-bottom: var(--ce-space-5);
  min-width: 0;
}

.stats {
  display: flex;
  flex-wrap: nowrap;
  gap: var(--ce-space-8);
  align-items: baseline;
}

.stats .session {
  margin-left: auto;
}

@media (max-width: 749px) {
  .stats {
    flex-wrap: wrap;
    gap: var(--ce-space-3) var(--ce-space-6);
  }

  .stats .session {
    margin-left: 0;
    flex-basis: 100%;
  }
}

.stat {
  display: flex;
  align-items: baseline;
  gap: var(--ce-space-2);
}

.stat-label {
  color: var(--ce-text-tertiary);
  font: var(--ce-type-caption);
  white-space: nowrap;
}

/* 会话摘要按钮：可点击展开/收起数据表 */
.session-summary {
  background: none;
  border: none;
  padding: 0;
  cursor: pointer;
  color: var(--ce-text-tertiary);
  font: var(--ce-type-caption);
  white-space: nowrap;
  transition: color var(--ce-dur-fast) var(--ce-ease-standard);
}

.session-summary:hover {
  color: var(--ce-text-secondary);
}

.session-summary:focus-visible {
  outline: 2px solid var(--ce-accent-bg);
  outline-offset: 2px;
  border-radius: var(--ce-radius-xs);
}

.stat-figures {
  display: flex;
  align-items: baseline;
  gap: var(--ce-space-1);
}

.stat-value {
  font: var(--ce-type-caption);
  font-variant-numeric: tabular-nums;
  color: var(--ce-text-primary);
  white-space: nowrap;
}

.stat-unit {
  font: var(--ce-type-caption);
  color: var(--ce-text-secondary);
}

.list-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ce-space-3);
  margin-bottom: var(--ce-space-3);
}

.search-box {
  display: flex;
  flex: 1;
  align-items: center;
  gap: var(--ce-space-2);
  min-width: 0;
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

/* 「全部断开…」：danger 文字按钮（EP text + danger 色），确认走 useConfirm。 */
.close-all {
  color: var(--ce-danger-fg);
}

.close-all:hover:not(.is-disabled) {
  color: var(--ce-danger-fg);
  background: var(--ce-danger-soft);
}

.connections-table {
  padding: 0;
  overflow: hidden;
}

.grid {
  display: grid;
  grid-template-columns: minmax(0, 2.2fr) minmax(0, 2fr) 110px 88px 88px 72px;
  align-items: center;
  gap: var(--ce-space-3);
  min-height: var(--ce-row-h);
  padding: 0 var(--ce-space-4);
}

.head {
  min-height: var(--ce-control-h);
  border-bottom: 1px solid var(--ce-divider);
  color: var(--ce-text-tertiary);
  font: var(--ce-type-caption);
}

.conn-list {
  max-height: 60vh;
  margin: 0;
  padding: 0;
  overflow: auto;
  list-style: none;
}

.conn-list .grid {
  border-top: 1px solid var(--ce-divider);
}

.conn-list .grid:first-child {
  border-top: 0;
}

.conn-list .grid:hover {
  background: var(--ce-fill-hover);
}

.num {
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.mono {
  font: var(--ce-type-mono);
}

.dim {
  color: var(--ce-text-secondary);
}

.host {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font: var(--ce-type-mono);
}

.route {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.rule,
.chain {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rule {
  font: var(--ce-type-callout);
}

.chain {
  color: var(--ce-accent-fg);
  font: var(--ce-type-caption);
  font-weight: 400;
}

.kind-tag {
  display: inline-flex;
  align-items: center;
  height: 22px;
  padding: 0 var(--ce-space-2);
  border-radius: var(--ce-radius-sm);
  background: var(--ce-fill-soft);
  color: var(--ce-text-secondary);
  font: var(--ce-type-caption);
  font-family: var(--ce-font-mono);
  white-space: nowrap;
}

.no-match {
  margin: 0;
  padding: var(--ce-space-4);
  color: var(--ce-text-tertiary);
  font: var(--ce-type-callout);
}

.truncated-notice {
  padding: var(--ce-space-2);
  text-align: center;
  color: var(--ce-text-tertiary);
  font: var(--ce-type-caption);
  font-weight: 400;
}

@media (max-width: 959px) {
  .grid {
    grid-template-columns: minmax(0, 1.6fr) minmax(0, 1.4fr) 88px 72px 72px;
  }

  .grid > :nth-child(3),
  .head > :nth-child(3) {
    display: none;
  }
}
</style>
