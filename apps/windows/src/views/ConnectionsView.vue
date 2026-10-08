<!-- src/views/ConnectionsView.vue - 连接列表：每 2s 轮询 get_connections -->
<script setup lang="ts">
import { computed } from "vue";
import { useAction } from "@/composables/useAction";
import { usePolling } from "@/composables/usePolling";
import { pollIntervalFor, useConnectionsStore } from "@/stores/connections";
import { formatBytes } from "@/utils/format";

const MAX_DISPLAY = 500;

const store = useConnectionsStore();
const closing = useAction();

const connections = computed(() => store.connections);
const connectionCount = computed(() => store.count);

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
  const r = await closing.run(() => store.closeAll(), { silent: true });
  if (r.ok) void tick();
}
</script>

<template>
  <div class="page connections-page">
    <div class="page-head">
      <h2 class="page-title">{{ $t("connections.title") }}</h2>
      <div class="page-head-right">
        <span class="conn-count" v-if="connectionCount > 0">{{ connectionCount }}</span>
        <span class="totals">
          {{ $t("connections.total_download") }}
          <b>{{ formatBytes(store.downloadTotal) }}</b>
          <span class="totals-sep">|</span>
          {{ $t("connections.total_upload") }}
          <b>{{ formatBytes(store.uploadTotal) }}</b>
        </span>
        <el-button type="danger" plain size="small" :loading="closing.busy.value" @click="onCloseAll">
          {{ $t("connections.close_all") }}
        </el-button>
      </div>
    </div>

    <el-table
      v-if="connections.length > 0"
      :data="connections"
      size="small"
      max-height="65vh"
      class="connections-table"
      :aria-label="$t('connections.title')"
    >
      <el-table-column
        prop="host"
        :label="$t('connections.host')"
        min-width="180"
        show-overflow-tooltip
      />
      <el-table-column prop="network" :label="$t('connections.network')" width="70" />
      <el-table-column
        prop="rule"
        :label="$t('connections.rule')"
        min-width="120"
        show-overflow-tooltip
      />
      <el-table-column :label="$t('connections.upload')" min-width="90" align="right">
        <template #default="{ row }">{{ formatBytes(row.upload) }}</template>
      </el-table-column>
      <el-table-column :label="$t('connections.download')" min-width="90" align="right">
        <template #default="{ row }">{{ formatBytes(row.download) }}</template>
      </el-table-column>
      <el-table-column :label="$t('connections.time')" min-width="90" align="right">
        <template #default="{ row }">{{ formatStart(row.start) }}</template>
      </el-table-column>
    </el-table>

    <el-empty v-else :description="$t('connections.empty')" />

    <div v-if="connectionCount > MAX_DISPLAY" class="truncated-notice" role="status">
      {{ $t("connections.truncated_notice", { max: MAX_DISPLAY, count: connectionCount }) }}
    </div>
  </div>
</template>

<style scoped>
.page-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 12px;
  margin-bottom: 16px;
}

.page-head .page-title {
  margin-bottom: 0;
}

.page-head-right {
  display: flex;
  align-items: center;
  gap: 16px;
}

.conn-count {
  font-size: 12px;
  color: var(--text-tertiary);
  background: var(--bg-soft);
  padding: 2px 10px;
  border-radius: 10px;
  white-space: nowrap;
}

.totals {
  font-size: 13px;
  color: var(--text-tertiary);
  white-space: nowrap;
}

/* 实时跳动的数字用等宽数字防抖动；分隔符弱化为纯留白（Flyme 靠间距不靠标点）。 */
.totals b {
  color: var(--text-secondary);
  font-weight: 500;
  font-variant-numeric: tabular-nums;
}

.totals-sep {
  margin: 0 4px;
  color: var(--border-subtle);
}

.truncated-notice {
  text-align: center;
  font-size: 12px;
  color: var(--text-tertiary);
  padding: 8px;
  background: var(--bg-soft);
  border: 1px solid var(--card-border);
  border-top: none;
  border-radius: 0 0 var(--r-md) var(--r-md);
}

.connections-table {
  --el-table-bg-color: transparent;
  --el-table-tr-bg-color: transparent;
  --el-table-header-bg-color: var(--bg-soft);
  --el-table-border-color: var(--border-subtle);
  --el-table-header-text-color: var(--text-tertiary);
  --el-table-text-color: var(--text-primary);
  --el-table-row-hover-bg-color: var(--interactive-hover);
  border: 1px solid var(--card-border);
  border-radius: var(--r-md);
  overflow: hidden;
}

/* 流量/时间列实时跳动：等宽数字防抖动。 */
.connections-table :deep(.el-table .cell) {
  font-variant-numeric: tabular-nums;
}
</style>
