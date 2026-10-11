<!-- src/views/DashboardView.vue - 首页：状态核心 + 实时流量
     设计约束：首页是「入口」，只做启停与状态；所有动作复用现有 store action。
     - 状态核心：启动/停止（主连接按钮）、重启、重载；代理模式/系统代理/TUN 三个快速开关
     - 实时流量：速率由 connections store 的累计值求差得出（沿用轮询策略，不另起轮询） -->
<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import CeSegmented from "@/components/ui/CeSegmented.vue";
import CeSparkline from "@/components/ui/CeSparkline.vue";
import CeStatusCore, { type CoreState } from "@/components/ui/CeStatusCore.vue";
import { useAction } from "@/composables/useAction";
import { usePolling } from "@/composables/usePolling";
import { useTunElevation } from "@/composables/useTunElevation";
import { useConfigStore } from "@/stores/config";
import { pollIntervalFor, useConnectionsStore } from "@/stores/connections";
import { useCoreStore } from "@/stores/core";
import { formatBytes, splitRate } from "@/utils/format";

const { t } = useI18n();
const core = useCoreStore();
const config = useConfigStore();
const connections = useConnectionsStore();

// 流量轮询：沿用 connections store 的自适应间隔；核心未运行时不请求。
usePolling(
  async () => {
    if (core.status.running) await connections.refresh();
  },
  () => pollIntervalFor(connections.count),
);

const running = computed(() => core.status.running);

// ---- 状态核心 ----
const coreError = computed(() => (core.status.status ?? "").startsWith("error:"));
const carrying = computed(() => config.systemProxy || config.tunEnabled);
const coreState = computed<CoreState>(() => {
  if (core.starting) return "starting";
  if (coreError.value) return "error";
  return running.value ? "running" : "stopped";
});
/** 「已连接」= 核心运行且系统代理或 TUN 至少开启一个；运行但没接管流量时如实说明。 */
const coreLabel = computed(() =>
  coreState.value === "running" && !carrying.value ? t("dashboard.status_idle") : undefined,
);
const versionLine = computed(() =>
  t("dashboard.version_port", { version: core.status.version ?? "—", port: config.config?.["mixed-port"] ?? "—" }),
);

// 动作守卫：restart/reload 无自带 starting 状态，用统一 busy 防连点；start/stop 由 store 兜底。
const coreActionBusy = ref(false);
const coreAction = useAction(coreActionBusy);
const startStop = useAction();
const onToggleCore = () =>
  startStop.run(() => (running.value && !coreError.value ? core.stop() : core.start()));
const onRestart = () => coreAction.run(() => core.restart());
const onReload = () => coreAction.run(() => core.reload());

const systemProxy = useAction();
const tun = useAction();
/** 代理模式分段：与代理页工具栏同一个 setProxyMode 动作，可就地快速切换。 */
const modeAction = useAction();
const modeOptions = computed(() =>
  ["rule", "global", "direct"].map((m) => ({ value: m, label: t(`tray.mode_${m}`) })),
);
const onModeChange = (mode: string) => modeAction.run(() => config.setProxyMode(mode));
/** 系统代理开关：走统一编排层（持久化意图 + 写注册表 + 托盘图标变色）。 */
const onSystemProxyChange = (val: boolean | string | number) =>
  systemProxy.run(() => config.setSystemProxy(Boolean(val)));
/** TUN 开关：与设置 › TUN 同一个 action；非管理员开启走 UAC 提权重启（确认后自动开启）。 */
const tunElevation = useTunElevation();
const onTunChange = (val: boolean | string | number) => {
  const on = Boolean(val);
  return tun.run(() => config.setTunMode(on), {
    success: on ? t("dashboard.tun_enabled") : t("dashboard.tun_disabled"),
    onError: on ? tunElevation.handleError : undefined,
  });
};

// ---- 实时流量 ----
const downRate = computed(() => splitRate(connections.rateDown[connections.rateDown.length - 1] ?? 0));
const upRate = computed(() => splitRate(connections.rateUp[connections.rateUp.length - 1] ?? 0));
const sessionLine = computed(() =>
  t("dashboard.session", {
    down: formatBytes(connections.downloadTotal),
    up: formatBytes(connections.uploadTotal),
    count: connections.count,
  }),
);
</script>

<template>
  <div class="page">
    <div class="dash-grid">
      <!-- 状态核心 -->
      <section class="ce-card core-card" :aria-label="$t('dashboard.core_status')">
        <CeStatusCore :state="coreState" :label="coreLabel" @toggle="onToggleCore">
          <span class="core-meta">{{ versionLine }}</span>
          <span class="core-links">
            <el-button size="small" :disabled="!running || coreActionBusy" @click="onRestart">{{ $t("dashboard.restart") }}</el-button>
            <el-button size="small" plain :disabled="!running || coreActionBusy" @click="onReload">{{ $t("dashboard.reload") }}</el-button>
          </span>
        </CeStatusCore>

        <div class="core-divider" aria-hidden="true"></div>

        <div class="core-controls">
          <div class="set-row">
            <span class="set-label">{{ $t("dashboard.proxy_mode") }}</span>
            <CeSegmented
              :model-value="config.proxyMode"
              :options="modeOptions"
              :ariaLabel="$t('dashboard.proxy_mode')"
              @update:model-value="(v) => onModeChange(String(v))"
            />
          </div>
          <div class="set-row">
            <span class="set-label">{{ $t("dashboard.system_proxy") }}</span>
            <el-switch
              :model-value="config.systemProxy"
              :aria-label="$t('dashboard.system_proxy')"
              :loading="systemProxy.busy.value"
              @change="onSystemProxyChange"
            />
          </div>
          <div class="set-row">
            <span class="set-label">{{ $t("dashboard.tun_mode") }}</span>
            <el-switch
              :model-value="config.tunEnabled"
              :aria-label="$t('dashboard.tun_mode')"
              :loading="tun.busy.value"
              @change="onTunChange"
            />
          </div>
        </div>
      </section>

      <!-- 实时流量 -->
      <section class="ce-card traffic-card" :aria-label="$t('dashboard.traffic_title')">
        <div class="rates">
          <div class="rate">
            <span class="rate-label">↓ {{ $t("ui.chart.down") }}</span>
            <span class="rate-figures">
              <span class="rate-value">{{ downRate[0] }}</span>
              <span class="rate-unit">{{ downRate[1] }}</span>
            </span>
          </div>
          <div class="rate">
            <span class="rate-label">↑ {{ $t("ui.chart.up") }}</span>
            <span class="rate-figures">
              <span class="rate-value">{{ upRate[0] }}</span>
              <span class="rate-unit">{{ upRate[1] }}</span>
            </span>
          </div>
          <p class="session-line">{{ sessionLine }}</p>
        </div>
        <CeSparkline
          :down="connections.rateDown"
          :up="connections.rateUp"
          :label="$t('dashboard.traffic_chart_label')"
        />
      </section>
    </div>
  </div>
</template>

<style scoped>
/* 两列 2:1 栅格；窗口宽度 < 960 时单列。 */
.dash-grid {
  display: grid;
  grid-template-columns: minmax(0, 2fr) minmax(0, 1fr);
  gap: var(--ce-space-4);
}

@media (max-width: 959px) {
  .dash-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}

/* ---- 状态核心 ---- */
.core-card {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ce-space-5);
}

.core-card > :first-child {
  flex: 1 1 320px;
}

.core-meta {
  font: var(--ce-type-callout);
  color: var(--ce-text-secondary);
  font-variant-numeric: tabular-nums;
}

.core-links {
  display: flex;
  gap: var(--ce-space-2);
  margin-top: var(--ce-space-1);
}

.core-divider {
  flex-basis: 100%;
  height: 1px;
  background: var(--ce-divider);
}

/* 底部三控件栅格：模式分段占首列（较宽），系统代理/TUN 次之；窄屏自动换行。 */
.core-controls {
  display: grid;
  flex-basis: 100%;
  grid-template-columns: minmax(0, 2fr) minmax(0, 1fr) minmax(0, 1fr);
  gap: var(--ce-space-4) var(--ce-space-5);
  align-items: center;
}

@media (max-width: 749px) {
  .core-controls {
    grid-template-columns: minmax(0, 1fr);
  }
}

.set-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ce-space-3);
  min-width: 0;
}

/* 代理模式只读展示当前模式（切换入口在代理页工具栏）。 */
.mode-value {
  font: var(--ce-type-body-strong);
  color: var(--ce-accent-fg);
}

.set-label {
  font: var(--ce-type-body-strong);
  color: var(--ce-text-primary);
}

/* ---- 实时流量 ---- */
.traffic-card {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-3);
  min-width: 0;
}

.rates {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--ce-space-3) var(--ce-space-8);
}

.rate {
  display: flex;
  align-items: baseline;
  gap: var(--ce-space-2);
}

.rate-label {
  font: var(--ce-type-caption);
  color: var(--ce-text-tertiary);
}

.rate-figures {
  display: flex;
  align-items: baseline;
  gap: var(--ce-space-1);
}

.rate-value {
  font: var(--ce-type-caption);
  color: var(--ce-text-primary);
  font-variant-numeric: tabular-nums;
}

.rate-unit {
  font: var(--ce-type-caption);
  color: var(--ce-text-secondary);
}

.session-line {
  margin: 0;
  margin-left: auto;
  font: var(--ce-type-caption);
  color: var(--ce-text-tertiary);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
</style>
