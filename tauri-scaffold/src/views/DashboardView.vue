<!-- src/views/DashboardView.vue - 概览：核心状态 + 核心控制 + 系统代理开关
     设计约束：概览只是「入口」，不做重管理 UI。
     - 状态卡：核心状态与「启动/停止 → 重载 → 重启」同卡，顺序排布
     - 设置卡：仅保留系统代理开关（订阅管理已移回独立「配置」页） -->
<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { ElMessage } from "element-plus";
import { proxyApi } from "@/api/proxy";
import { useConfigStore } from "@/stores/config";
import { useCoreStore } from "@/stores/core";
import { useProxyStore } from "@/stores/proxy";

const core = useCoreStore();
const config = useConfigStore();
const proxyStore = useProxyStore();

// Dashboard 需要当前节点/延迟：进入页面且核心运行时加载一次代理组，
// 复用「代理」页同一个 store，不重复建数据源；核心从停止→运行（含在
// 本页启动核心）后也要刷新，保证「界面状态 = 应用状态」。
onMounted(() => {
  if (core.status.running) void proxyStore.loadGroups();
});
watch(
  () => core.status.running,
  (running) => {
    if (running) void proxyStore.loadGroups();
  }
);

const running = computed(() => core.status.running);
// 核心控制动作 in-flight 守卫：restart/reload 无自带的 starting/stopping 状态，
// 用统一 busy 标志防止连点重复触发；start/stop 已由 store 的 starting/stopping 兜底。
const coreActionBusy = ref(false);
const statusKey = computed(() =>
  core.starting
    ? "dashboard.starting"
    : core.stopping
      ? "dashboard.stopping"
      : running.value
        ? "dashboard.running"
        : "dashboard.stopped"
);

/** 当前节点所处的组。
 *  - 全局模式：GLOBAL 组的当前选中；
 *  - 规则模式：优先选「当前选中是真实节点」的组（排除 DIRECT/REJECT/PASS 占位），
 *    顺序对齐「代理」页（扶梯出行→人工智能→影音视听→人工优选→自动优选），
 *    避免某组停在占位节点时把它误当出口；全都占位则回退第一组。
 *  - 直连模式：无代理节点。
 * 取到的 `now` 即为真实出口选中，确保概览与代理页一致。 */
const PLACEHOLDER_NODES = new Set(["DIRECT", "REJECT", "PASS"]);
const currentGroup = computed(() => {
  const groups = proxyStore.groups;
  if (!groups.length) return undefined;
  if (config.proxyMode === "direct") return undefined;
  if (config.proxyMode === "global") {
    return groups.find((g) => g.name === "GLOBAL") ?? groups[0];
  }
  const order = ["扶梯出行", "人工智能", "影音视听", "人工优选", "自动优选", "GLOBAL"];
  const ranked = [...groups].sort((a, b) => {
    const ai = order.indexOf(a.name);
    const bi = order.indexOf(b.name);
    return (ai === -1 ? 99 : ai) - (bi === -1 ? 99 : bi);
  });
  // 优先「当前选中非占位」的组：真实出口节点优先展示，自动优选不会被占位组吃掉
  const withReal = ranked.find((g) => g.now && !PLACEHOLDER_NODES.has(g.now));
  return withReal ?? ranked[0];
});
const currentNode = computed(() => {
  if (config.proxyMode === "direct") return "—";
  return currentGroup.value?.now ?? "—";
});
/** 当前节点延迟：复用代理页已测的组延迟；为空时整行不显示（避免 "—" 困惑）。 */
const currentLatency = computed(() => {
  const g = currentGroup.value;
  if (!g) return null;
  const d = proxyStore.delays[g.name];
  return d == null ? null : `${d} ms`;
});

/** 系统代理开关：走统一编排层（持久化意图 + 写注册表 + 托盘图标变色），
 *  成功后同步本地 store，避免下次整包保存时把该字段覆盖回 false。 */
async function onSystemProxyChange(val: boolean) {
  try {
    await proxyApi.setSystemProxy(val);
    if (config.config) config.config["system-proxy"] = val;
  } catch (e) {
    ElMessage.error(String(e));
  }
}

async function onRestart() {
  if (coreActionBusy.value) return;
  coreActionBusy.value = true;
  try {
    await core.restart();
  } catch (e) {
    ElMessage.error(String(e));
  } finally {
    coreActionBusy.value = false;
  }
}

async function onReload() {
  if (coreActionBusy.value) return;
  coreActionBusy.value = true;
  try {
    await core.reload();
  } catch (e) {
    ElMessage.error(String(e));
  } finally {
    coreActionBusy.value = false;
  }
}

async function onStart() {
  try {
    await core.start();
  } catch (e) {
    ElMessage.error(String(e));
  }
}

async function onStop() {
  try {
    await core.stop();
  } catch (e) {
    ElMessage.error(String(e));
  }
}
</script>

<template>
  <div class="page">
    <h2 class="page-title">{{ $t("dashboard.title") }}</h2>

    <!-- 状态卡：核心状态 + 核心控制按钮（同卡，顺序：启动/停止 → 重载 → 重启） -->
    <el-card shadow="never" class="status-card">
      <template #header>
        <div class="status-head">
          <span>{{ $t("dashboard.core_status") }}</span>
          <span class="status-pill" :class="{ running: running }">
            <span class="status-dot" aria-hidden="true"></span>
            {{ $t(statusKey) }}
          </span>
        </div>
      </template>

      <div class="status-grid">
        <div class="stat-item">
          <div class="stat-label">{{ $t("dashboard.current_node") }}</div>
          <div class="stat-value node-value" :title="currentNode">
            <span class="node-name">{{ currentNode }}</span>
            <span v-if="currentLatency" class="node-latency">{{ currentLatency }}</span>
          </div>
        </div>
        <div class="stat-item">
          <div class="stat-label">{{ $t("dashboard.proxy_mode") }}</div>
          <div class="stat-value">{{ $t("tray.mode_" + config.proxyMode) }}</div>
        </div>
        <div class="stat-item">
          <div class="stat-label">{{ $t("dashboard.core_version") }}</div>
          <div class="stat-value">{{ core.status.version ?? "—" }}</div>
        </div>
      </div>

      <!-- 核心控制与状态同卡，均分三列：启动/停止 → 重启核心 → 重载配置 -->
      <div class="core-actions">
        <el-button
          v-if="!running"
          type="primary"
          class="core-btn"
          :loading="core.starting"
          @click="onStart"
        >
          {{ $t("dashboard.start") }}
        </el-button>
        <el-button
          v-else
          type="danger"
          plain
          class="core-btn"
          :loading="core.stopping"
          @click="onStop"
        >
          {{ $t("dashboard.stop") }}
        </el-button>
        <el-button class="core-btn" :loading="coreActionBusy" :disabled="!running || coreActionBusy" @click="onRestart">
          {{ $t("dashboard.restart") }}
        </el-button>
        <el-button class="core-btn" :loading="coreActionBusy" :disabled="!running || coreActionBusy" @click="onReload">
          {{ $t("dashboard.reload") }}
        </el-button>
      </div>
    </el-card>

    <!-- 设置卡：仅系统代理开关（订阅管理已移回「配置」页） -->
    <el-card shadow="never" class="settings-card">
      <div class="set-row">
        <div class="set-info">
          <div class="set-label">{{ $t("dashboard.system_proxy") }}</div>
          <div class="set-hint">{{ $t("dashboard.system_proxy_hint") }}</div>
        </div>
        <el-switch :model-value="config.systemProxy" @change="onSystemProxyChange" />
      </div>
    </el-card>
  </div>
</template>

<style scoped>
.status-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-weight: 500;
}

/* 状态指示由全局 .status-pill/.status-dot/.running 提供（styles.css），
 * 本页不再重复定义，保证与 Logs 页语义色一致。 */

.status-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 12px;
}

/* 小窗口：三列过窄，降为上下堆叠的行式（保留分隔线语义改为上边框）。 */
@media (max-width: 640px) {
  .status-grid {
    grid-template-columns: 1fr;
    row-gap: 4px;
  }
  .stat-item + .stat-item {
    border-left: none;
    border-top: 1px solid var(--card-border);
  }
}

.stat-item {
  padding: var(--space-1) 0 var(--space-1) var(--space-4);
}

.stat-item + .stat-item {
  border-left: 1px solid var(--card-border);
}

.stat-label {
  font-size: 12px;
  color: var(--text-tertiary);
  margin-bottom: 6px;
}

.stat-value {
  font-size: 18px;
  font-weight: 600;
  color: var(--text-primary);
}

/* 当前节点：与其它 stat-value 同字号同字重（协调），
 * 名称超长省略、完整名走 title；延迟作为次级信息跟随。 */
.node-value {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
}

.node-value .node-name {
  overflow: hidden;
  text-overflow: ellipsis;
}

.node-latency {
  flex: none;
  font-size: 12px;
  font-weight: 500;
  color: var(--text-tertiary);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.status-card {
  /* 单值！EP 头部用 calc(var(--el-card-padding) - 2px)，双值会让 calc
     失效、头部内边距归零（核心状态/运行中贴边的根因）。 */
  --el-card-padding: var(--space-5);
}

.core-actions {
  margin-top: 20px;
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 12px;
}

/* 小窗口（~560px）下三列按钮过窄：换行为自动列宽，避免文案截断。 */
@media (max-width: 640px) {
  .core-actions {
    grid-template-columns: 1fr;
  }
}

.core-btn {
  width: 100%;
  margin-left: 0 !important;
}

.settings-card {
  margin-top: 16px;
}

.set-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}

.set-info {
  min-width: 0;
}

.set-label {
  font-weight: 500;
  font-size: 14px;
  color: var(--text-primary);
}

.set-hint {
  margin-top: 4px;
  font-size: 12px;
  color: var(--text-tertiary);
}
</style>
