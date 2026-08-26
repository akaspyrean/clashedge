<!-- src/views/ConnectionsView.vue - 连接列表：每 2s 轮询 get_connections -->
<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import {
  connectionsApi,
  type ConnectionInfo,
} from "@/api/connections";

const MAX_DISPLAY = 500;

function pollIntervalFor(count: number): number {
  if (count < 200) return 2000;
  if (count < 1000) return 3000;
  if (count < 5000) return 5000;
  return 8000;
}

const connections = ref<ConnectionInfo[]>([]);
const connectionCount = ref(0);
const truncated = ref(false);
const downloadTotal = ref(0);
const uploadTotal = ref(0);
let timer: number | undefined;

// 在途请求守卫：上一轮请求尚未返回时跳过本轮，避免 2s 定时器与慢请求堆叠
// 造成重复拉取 / 乱序覆盖。
let inFlight = false;
// 关闭全部 in-flight 守卫：连点时不重复发起 closeAll。
const closingAll = ref(false);

/** B / KB / MB / GB / TB，保留 1-2 位小数。 */
function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(
    units.length - 1,
    Math.floor(Math.log(bytes) / Math.log(1024)),
  );
  const value = bytes / Math.pow(1024, i);
  const digits = value >= 100 ? 0 : value >= 10 ? 1 : 2;
  return `${value.toFixed(digits)} ${units[i]}`;
}

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

async function refresh() {
  if (inFlight) return;
  inFlight = true;
  try {
    const data = await connectionsApi.list();
    const all = data.connections ?? [];
    // P2：后端已裁剪到前 500 条；connectionCount 用真实总数 total 驱动
    // 轮询间隔与截断提示，truncated 标记是否渲染截断提示。
    connectionCount.value = data.total ?? all.length;
    truncated.value = data.truncated ?? false;
    connections.value = all;
    downloadTotal.value = data.download_total ?? 0;
    uploadTotal.value = data.upload_total ?? 0;
  } catch {
  } finally {
    inFlight = false;
    // 根据新连接数动态调整下一轮轮询间隔
    restartPolling();
  }
}

async function onCloseAll() {
  if (closingAll.value) return;
  closingAll.value = true;
  try {
    await connectionsApi.closeAll();
    void refresh();
  } catch {
    // 静默处理。
  } finally {
    closingAll.value = false;
  }
}

function startPolling() {
  if (timer !== undefined) return;
  const interval = pollIntervalFor(connectionCount.value);
  timer = window.setInterval(() => {
    void refresh();
  }, interval);
}

function restartPolling() {
  if (timer !== undefined) {
    window.clearInterval(timer);
    timer = undefined;
  }
  startPolling();
}

function stopPolling() {
  if (timer !== undefined) {
    window.clearInterval(timer);
    timer = undefined;
  }
}

// 页面隐藏（最小化/切走）时暂停 2s 轮询，恢复可见时立即拉一次并重启轮询。
function onVisibilityChange() {
  if (document.hidden) {
    stopPolling();
  } else {
    void refresh();
    startPolling();
  }
}

onMounted(() => {
  void refresh();
  startPolling();
  document.addEventListener("visibilitychange", onVisibilityChange);
});

onUnmounted(() => {
  stopPolling();
  document.removeEventListener("visibilitychange", onVisibilityChange);
});
</script>

<template>
  <div class="page connections-page">
    <div class="page-head">
      <h2 class="page-title">{{ $t("connections.title") }}</h2>
      <div class="page-head-right">
        <span class="conn-count" v-if="connectionCount > 0">{{ connectionCount }}</span>
        <span class="totals">
          {{ $t("connections.total_download") }}
          <b>{{ formatBytes(downloadTotal) }}</b>
          <span class="totals-sep">|</span>
          {{ $t("connections.total_upload") }}
          <b>{{ formatBytes(uploadTotal) }}</b>
        </span>
        <el-button type="danger" plain size="small" :loading="closingAll" @click="onCloseAll">
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
    >
      <el-table-column type="index" width="50" />
      <el-table-column
        prop="host"
        :label="$t('connections.host')"
        min-width="200"
        show-overflow-tooltip
      />
      <el-table-column prop="network" :label="$t('connections.network')" width="90" />
      <el-table-column
        prop="rule"
        :label="$t('connections.rule')"
        min-width="150"
        show-overflow-tooltip
      />
      <el-table-column :label="$t('connections.upload')" width="110" align="right">
        <template #default="{ row }">{{ formatBytes(row.upload) }}</template>
      </el-table-column>
      <el-table-column :label="$t('connections.download')" width="110" align="right">
        <template #default="{ row }">{{ formatBytes(row.download) }}</template>
      </el-table-column>
      <el-table-column :label="$t('connections.time')" width="100" align="right">
        <template #default="{ row }">{{ formatStart(row.start) }}</template>
      </el-table-column>
    </el-table>

    <el-empty v-else :description="$t('connections.empty')" />

    <div v-if="connectionCount > MAX_DISPLAY" class="truncated-notice">
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

.totals b {
  color: var(--text-primary);
  font-weight: 500;
}

.totals-sep {
  margin: 0 10px;
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
</style>
