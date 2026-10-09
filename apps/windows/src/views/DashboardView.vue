<!-- src/views/DashboardView.vue - 首页：状态核心 + 实时流量 + 快捷切换 + 当前订阅
     设计约束：首页是「入口」，不做重管理 UI；所有动作复用现有 store action，不新增后端接口。
     - 状态核心：启动/停止（主连接按钮）、重启、重载、代理模式、系统代理、TUN
     - 实时流量：速率由 connections store 的累计值求差得出（沿用轮询策略，不另起轮询）
     - 快捷切换：策略组下拉 + 节点格；自动优选组只读 -->
<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { Zap, ArrowRight } from "lucide-vue-next";
import CeNodeTile from "@/components/ui/CeNodeTile.vue";
import CeSegmented from "@/components/ui/CeSegmented.vue";
import CeSparkline from "@/components/ui/CeSparkline.vue";
import CeStatusCore, { type CoreState } from "@/components/ui/CeStatusCore.vue";
import { useAction } from "@/composables/useAction";
import { useNodeRetest } from "@/composables/useNodeRetest";
import { usePolling } from "@/composables/usePolling";
import { resolveGroupId, sortRuleGroups } from "@/constants/groups";
import { useConfigStore } from "@/stores/config";
import { pollIntervalFor, useConnectionsStore } from "@/stores/connections";
import { useCoreStore } from "@/stores/core";
import { useProfilesStore } from "@/stores/profiles";
import { useProxyStore } from "@/stores/proxy";
import { formatBytes, parseUserinfo, splitRate } from "@/utils/format";

const { t } = useI18n();
const core = useCoreStore();
const config = useConfigStore();
const proxyStore = useProxyStore();
const connections = useConnectionsStore();
const profiles = useProfilesStore();

// 进入页面且核心运行时加载一次代理组与订阅列表（复用「代理」「订阅」页同一个 store）；
// 核心从停止→运行（含在本页启动核心）后也要刷新，保证「界面状态 = 应用状态」。
onMounted(() => {
  void profiles.list();
  if (core.status.running) void proxyStore.loadGroups();
});
watch(
  () => core.status.running,
  (running) => {
    if (running) void proxyStore.loadGroups();
  },
);

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
const startStop = useAction(ref(false));
const onToggleCore = () =>
  startStop.run(() => (running.value && !coreError.value ? core.stop() : core.start()));
const onRestart = () => coreAction.run(() => core.restart());
const onReload = () => coreAction.run(() => core.reload());

const modeAction = useAction();
const systemProxy = useAction();
const tun = useAction();
const modeOptions = computed(() =>
  ["rule", "global", "direct"].map((m) => ({ value: m, label: t(`tray.mode_${m}`) })),
);
const onModeChange = (mode: string) => modeAction.run(() => config.setProxyMode(mode));
/** 系统代理开关：走统一编排层（持久化意图 + 写注册表 + 托盘图标变色）。 */
const onSystemProxyChange = (val: boolean | string | number) =>
  systemProxy.run(() => config.setSystemProxy(Boolean(val)));
/** TUN 开关：与设置 › TUN 同一个 action，提权流程不变。 */
const onTunChange = (val: boolean | string | number) =>
  tun.run(() => config.setTunMode(Boolean(val)), { success: true });

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

// ---- 快捷切换 ----
const groups = computed(() => sortRuleGroups(proxyStore.groups));
/** 默认显示「扶梯出行」当前指向的组（如 → 人工优选）；它指向的是节点则显示它自己。 */
const defaultGroupName = computed(() => {
  const entry = groups.value.find((g) => resolveGroupId(g.name) === "proxy") ?? groups.value[0];
  if (!entry) return "";
  return groups.value.some((g) => g.name === entry.now) ? entry.now : entry.name;
});
const pickedGroup = ref<string | null>(null);
const activeGroup = computed(
  () => groups.value.find((g) => g.name === (pickedGroup.value ?? defaultGroupName.value)) ?? groups.value[0],
);
const readonlyGroup = computed(() => resolveGroupId(activeGroup.value?.name ?? "") === "auto");
const tiles = computed(() => {
  const g = activeGroup.value;
  if (!g) return [];
  const leaf = ["manual", "auto"].includes(resolveGroupId(g.name));
  return g.all
    .filter((p) => !(leaf && p === "DIRECT"))
    .map((name) => {
      const d = proxyStore.nodeDelays[name];
      return {
        name,
        ms: d ?? null,
        timeout: d === null,
        testing: testingNodes.value.has(name),
        selected: g.now === name,
      };
    });
});
const { testing: testingNodes, retest: onRetest } = useNodeRetest();
const select = useAction();
const onSelectNode = (name: string) =>
  activeGroup.value && select.run(() => proxyStore.select(activeGroup.value!.name, name));
const onTestGroup = () => activeGroup.value && proxyStore.testGroupProxies(activeGroup.value.name);

// ---- 当前订阅 ----
const sub = computed(() => {
  const p = profiles.activeProfile;
  if (!p) return null;
  const info = parseUserinfo(p.userinfo);
  const pct = info && info.total > 0 ? Math.min(100, Math.round((info.used / info.total) * 100)) : null;
  let expire = "";
  let expireTone = "";
  if (info?.expire) {
    const days = Math.ceil((info.expire * 1000 - Date.now()) / 86_400_000);
    expire =
      days < 0
        ? t("dashboard.sub_expired")
        : `${new Date(info.expire * 1000).toLocaleDateString()} · ${t("dashboard.sub_days_left", { n: days })}`;
    expireTone = days < 0 ? "is-danger" : days <= 7 ? "is-warning" : "";
  } else if (info?.expire === 0) {
    expire = t("dashboard.sub_never");
  }
  return {
    name: p.name,
    used: info ? formatBytes(info.used) : "",
    total: info ? (info.total ? formatBytes(info.total) : t("profiles.traffic_unlimited")) : "",
    pct,
    expire,
    expireTone,
  };
});
</script>

<template>
  <div class="page">
    <h2 class="page-title">{{ $t("dashboard.title") }}</h2>

    <div class="dash-grid">
      <!-- 状态核心 -->
      <section class="ce-card core-card" :aria-label="$t('dashboard.core_status')">
        <CeStatusCore :state="coreState" :label="coreLabel" @toggle="onToggleCore">
          <span class="core-meta">{{ versionLine }}</span>
          <span class="core-links">
            <el-button text :disabled="!running || coreActionBusy" @click="onRestart">{{ $t("dashboard.restart") }}</el-button>
            <el-button text :disabled="!running || coreActionBusy" @click="onReload">{{ $t("dashboard.reload") }}</el-button>
          </span>
        </CeStatusCore>

        <div class="core-divider" aria-hidden="true"></div>

        <div class="core-controls">
          <div class="control-block">
            <span class="control-label">{{ $t("dashboard.proxy_mode") }}</span>
            <CeSegmented
              :model-value="config.proxyMode"
              :options="modeOptions"
              :ariaLabel="$t('dashboard.proxy_mode')"
              @update:model-value="(v) => onModeChange(String(v))"
            />
          </div>
          <div class="set-row">
            <div class="set-info">
              <div class="set-label">{{ $t("dashboard.system_proxy") }}</div>
            </div>
            <el-switch
              :model-value="config.systemProxy"
              :aria-label="$t('dashboard.system_proxy')"
              :loading="systemProxy.busy.value"
              @change="onSystemProxyChange"
            />
          </div>
          <div class="set-row">
            <div class="set-info">
              <div class="set-label">{{ $t("dashboard.tun_mode") }}</div>
              <div class="set-hint">{{ $t("dashboard.tun_hint") }}</div>
            </div>
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
        <h3 class="card-title">{{ $t("dashboard.traffic_title") }}</h3>
        <div class="rates">
          <div class="rate">
            <span class="rate-label">↓ {{ $t("ui.chart.down") }}</span>
            <span class="rate-value">{{ downRate[0] }}</span>
            <span class="rate-unit">{{ downRate[1] }}</span>
          </div>
          <div class="rate">
            <span class="rate-label">↑ {{ $t("ui.chart.up") }}</span>
            <span class="rate-value">{{ upRate[0] }}</span>
            <span class="rate-unit">{{ upRate[1] }}</span>
          </div>
        </div>
        <CeSparkline
          :down="connections.rateDown"
          :up="connections.rateUp"
          :label="$t('dashboard.traffic_chart_label')"
        />
        <p class="session-line">{{ sessionLine }}</p>
      </section>

      <!-- 快捷切换 -->
      <section class="ce-card quick-card" :aria-label="$t('dashboard.quick_title')">
        <div class="quick-head">
          <h3 class="card-title">{{ $t("dashboard.quick_title") }}</h3>
          <el-select
            v-if="groups.length"
            :model-value="activeGroup?.name"
            :aria-label="$t('dashboard.quick_group')"
            class="group-select"
            @change="(v: string) => (pickedGroup = v)"
          >
            <el-option v-for="g in groups" :key="g.name" :label="g.name" :value="g.name" />
          </el-select>
          <span class="quick-spacer"></span>
          <el-button text :disabled="!activeGroup || proxyStore.testingNodes" :loading="proxyStore.testingNodes" @click="onTestGroup">
            <Zap :size="16" :stroke-width="1.75" aria-hidden="true" />{{ $t("dashboard.quick_test") }}
          </el-button>
          <a class="quick-all" href="#/proxies">
            {{ $t("dashboard.quick_all") }}<ArrowRight :size="16" :stroke-width="1.75" aria-hidden="true" />
          </a>
        </div>
        <p v-if="readonlyGroup" class="quick-note">{{ $t("dashboard.quick_readonly") }}</p>
        <div v-if="tiles.length" class="tile-grid">
          <CeNodeTile
            v-for="n in tiles"
            :key="n.name"
            :name="n.name"
            :ms="n.ms"
            :timeout="n.timeout"
            :testing="n.testing"
            :selected="n.selected"
            :readonly="readonlyGroup"
            @select="onSelectNode(n.name)"
            @retest="onRetest(n.name)"
          />
        </div>
        <p v-else class="quick-empty">{{ $t("dashboard.quick_empty") }}</p>
      </section>

      <!-- 当前订阅 -->
      <section class="ce-card sub-card" :aria-label="$t('dashboard.sub_title')">
        <h3 class="card-title">{{ $t("dashboard.sub_title") }}</h3>
        <template v-if="sub">
          <div class="sub-name">{{ sub.name }}</div>
          <template v-if="sub.used">
            <div class="sub-row">
              <span class="sub-key">{{ $t("dashboard.sub_used") }}</span>
              <span class="sub-val">{{ sub.used }} <span class="sub-total">/ {{ sub.total }}</span></span>
            </div>
            <div
              v-if="sub.pct !== null"
              class="sub-bar"
              role="progressbar"
              :aria-valuenow="sub.pct"
              aria-valuemin="0"
              aria-valuemax="100"
              :aria-label="$t('dashboard.sub_used')"
            >
              <i :style="{ width: sub.pct + '%' }"></i>
            </div>
          </template>
          <div v-if="sub.expire" class="sub-row">
            <span class="sub-key">{{ $t("dashboard.sub_expire") }}</span>
            <span class="sub-val" :class="sub.expireTone">{{ sub.expire }}</span>
          </div>
        </template>
        <p v-else class="quick-empty">{{ $t("dashboard.sub_none") }}</p>
        <a class="quick-all sub-link" href="#/profiles">
          {{ $t("dashboard.sub_manage") }}<ArrowRight :size="16" :stroke-width="1.75" aria-hidden="true" />
        </a>
      </section>
    </div>
  </div>
</template>

<style scoped>
/* 两行 2:1 栅格；窗口宽度 < 960 时单列。 */
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

.card-title {
  margin: 0;
  font: var(--ce-type-title-3);
  color: var(--ce-text-primary);
}

/* ---- 状态核心 ---- */
.core-card {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ce-space-6);
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
  gap: var(--ce-space-1);
  margin-left: calc(var(--ce-space-3) * -1);
}

.core-divider {
  align-self: stretch;
  width: 1px;
  background: var(--ce-divider);
}

@media (max-width: 749px) {
  .core-divider {
    display: none;
  }
}

.core-controls {
  display: flex;
  flex: 1 1 260px;
  flex-direction: column;
  gap: var(--ce-space-3);
  min-width: 0;
}

.control-block {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--ce-space-2);
}

.control-label,
.rate-label,
.sub-key {
  font: var(--ce-type-caption);
  color: var(--ce-text-tertiary);
}

.set-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ce-space-4);
}

.set-info {
  min-width: 0;
}

.set-label {
  font: var(--ce-type-body);
  color: var(--ce-text-primary);
}

.set-hint {
  font: var(--ce-type-caption);
  font-weight: 400;
  color: var(--ce-text-tertiary);
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
  gap: var(--ce-space-5);
}

.rate {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
}

.rate-value {
  font: var(--ce-type-display);
  font-variant-numeric: tabular-nums;
  color: var(--ce-text-primary);
}

.rate-unit {
  font: var(--ce-type-callout);
  color: var(--ce-text-secondary);
  font-variant-numeric: tabular-nums;
}

.session-line {
  margin: 0;
  font: var(--ce-type-callout);
  color: var(--ce-text-secondary);
  font-variant-numeric: tabular-nums;
}

/* ---- 快捷切换 ---- */
.quick-card {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-3);
  min-width: 0;
}

.quick-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ce-space-3);
}

.group-select {
  width: 160px;
}

.quick-spacer {
  flex: 1;
}

.quick-note {
  margin: 0;
  font: var(--ce-type-callout);
  color: var(--ce-text-secondary);
}

.quick-empty {
  margin: 0;
  font: var(--ce-type-callout);
  color: var(--ce-text-tertiary);
}

.tile-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: var(--ce-space-3);
}

.quick-all {
  display: inline-flex;
  align-items: center;
  gap: var(--ce-space-1);
  font: var(--ce-type-callout);
  font-weight: 500;
  color: var(--ce-accent-fg);
  text-decoration: none;
}

.quick-all:hover {
  text-decoration: underline;
}

/* ---- 当前订阅 ---- */
.sub-card {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-3);
  min-width: 0;
}

.sub-name {
  font: var(--ce-type-title-3);
  color: var(--ce-text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sub-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--ce-space-3);
}

.sub-val {
  font: var(--ce-type-callout);
  color: var(--ce-text-primary);
  font-variant-numeric: tabular-nums;
}

.sub-val.is-warning {
  color: var(--ce-warning-fg);
}

.sub-val.is-danger {
  color: var(--ce-danger-fg);
}

.sub-total {
  color: var(--ce-text-tertiary);
}

.sub-bar {
  height: 6px;
  overflow: hidden;
  border-radius: var(--ce-radius-full);
  background: var(--ce-fill-soft);
}

.sub-bar > i {
  display: block;
  height: 100%;
  border-radius: var(--ce-radius-full);
  background: var(--ce-accent-bg);
}

.sub-link {
  margin-top: auto;
}
</style>
