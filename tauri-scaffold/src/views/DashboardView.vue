<!-- src/views/DashboardView.vue - 概览：核心状态 + 核心控制 + 系统代理开关
     设计约束：概览只是「入口」，不做重管理 UI。
     - 状态卡：核心状态与「启动/停止 → 重载 → 重启」同卡，顺序排布
     - 设置卡：仅保留系统代理开关（订阅管理已移回独立「配置」页） -->
<script setup lang="ts">
import { computed, ref } from "vue";
import { ElMessage } from "element-plus";
import { proxyApi } from "@/api/proxy";
import { useConfigStore } from "@/stores/config";
import { useCoreStore } from "@/stores/core";

const core = useCoreStore();
const config = useConfigStore();

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
          <div class="stat-label">{{ $t("dashboard.core_version") }}</div>
          <div class="stat-value">{{ core.status.version ?? "—" }}</div>
        </div>
        <div class="stat-item">
          <div class="stat-label">{{ $t("dashboard.mixed_port") }}</div>
          <div class="stat-value">{{ config.mixedPort }}</div>
        </div>
        <div class="stat-item">
          <div class="stat-label">{{ $t("dashboard.proxy_mode") }}</div>
          <div class="stat-value">{{ $t("tray.mode_" + config.proxyMode) }}</div>
        </div>
      </div>

      <!-- 核心控制与状态同卡，均分三列：启动/停止 → 重启核心 → 重载配置 -->
      <div class="core-actions">
        <el-button
          v-if="!running"
          type="primary"
          class="core-btn"
          :loading="core.starting"
          @click="core.start()"
        >
          {{ $t("dashboard.start") }}
        </el-button>
        <el-button
          v-else
          type="danger"
          plain
          class="core-btn"
          :loading="core.stopping"
          @click="core.stop()"
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

/* 状态指示：小圆点 + 文字，替代高饱和状态灯（设计规范 §8）。 */
.status-pill {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  font-size: 13px;
  font-weight: 500;
  color: var(--text-secondary);
}

.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-tertiary);
}

.status-pill.running {
  color: var(--done);
}

.status-pill.running .status-dot {
  background: var(--done);
}

.status-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 12px;
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
