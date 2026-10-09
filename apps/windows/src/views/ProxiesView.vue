<!-- src/views/ProxiesView.vue - 代理：策略组列表 + 节点格（主从双栏）
     - 左栏：策略组（名称 / 类型 / 当前节点）；右栏：选中组的节点格 + 地区筛选 + 按延迟排序
     - 自动优选（URLTest）组节点格只读：由测速自动选择
     - 模式过滤保持不变：rule 显示 5 组并隐藏 GLOBAL；global 只显示 GLOBAL；direct 显示空状态说明
     - 窗口 < 960：左栏改为顶部横向芯片 -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { RefreshCw, Search, Zap, ArrowDownWideNarrow } from "lucide-vue-next";
import type { ProxyGroup } from "@/api/proxy";
import CeNodeTile from "@/components/ui/CeNodeTile.vue";
import CeSegmented from "@/components/ui/CeSegmented.vue";
import { useAction } from "@/composables/useAction";
import { useNodeRetest } from "@/composables/useNodeRetest";
import { resolveGroupId, sortRuleGroups } from "@/constants/groups";
import { useConfigStore } from "@/stores/config";
import { useCoreStore } from "@/stores/core";
import { useProxyStore } from "@/stores/proxy";
import { regionOf, type RegionId } from "@/utils/region";

const { t } = useI18n();
const proxyStore = useProxyStore();
const configStore = useConfigStore();
const coreStore = useCoreStore();

// 节点选中 in-flight 守卫：连点节点时只允许一个 select 在途，避免乱序覆盖。
const select = useAction();
const modeAction = useAction();
const { testing: testingNodes, retest: onRetest } = useNodeRetest();

// mihomo 官方模板仅这三值；script 是 Clash Premium 遗留，后端会拒绝。
const modeOptions = computed(() =>
  ["rule", "global", "direct"].map((m) => ({ value: m, label: t(`tray.mode_${m}`) })),
);
/** 切换全局代理模式：走统一编排层（持久化 + 实时 PATCH 核心 + 托盘刷新）。 */
const onModeChange = (mode: string) => modeAction.run(() => configStore.setProxyMode(mode));

/** 按当前模式筛选可见组并排序：rule 显示 5 组（隐藏 GLOBAL）、global 只显示 GLOBAL、direct 无组。 */
const visibleGroups = computed<ProxyGroup[]>(() => {
  const mode = configStore.proxyMode;
  if (mode === "global") return proxyStore.groups.filter((g) => resolveGroupId(g.name) === "global");
  if (mode === "direct") return [];
  return sortRuleGroups(proxyStore.groups);
});

const selectedName = ref<string | null>(null);
const current = computed(
  () => visibleGroups.value.find((g) => g.name === selectedName.value) ?? visibleGroups.value[0],
);
const isAuto = (g: ProxyGroup) => resolveGroupId(g.name) === "auto" || g.type === "URLTest";
const isLeaf = (g: ProxyGroup) => ["manual", "auto"].includes(resolveGroupId(g.name));
const readonlyGroup = computed(() => (current.value ? isAuto(current.value) : false));

/** 叶子组（人工优选/自动优选）屏蔽内置 DIRECT，只显示真实节点。 */
function nodesOf(g: ProxyGroup): string[] {
  return isLeaf(g) ? g.all.filter((p) => p !== "DIRECT") : g.all;
}

/** 节点延迟：节点名优先；若该名字本身是一个组（如 扶梯出行 → 人工优选），用组延迟。 */
function delayFor(name: string): number | null | undefined {
  const d = proxyStore.nodeDelays[name];
  if (d !== undefined) return d;
  return name in proxyStore.delays ? proxyStore.delays[name] : undefined;
}

// ---- 搜索 / 地区 / 排序 ----
const query = ref("");
const region = ref<RegionId | "all">("all");
const sortByLatency = ref(false);
const searchInput = ref<HTMLInputElement | null>(null);

const groupNodes = computed(() => (current.value ? nodesOf(current.value) : []));
const regionChips = computed(() => {
  const counts = new Map<RegionId, number>();
  for (const n of groupNodes.value) counts.set(regionOf(n), (counts.get(regionOf(n)) ?? 0) + 1);
  const chips: { id: RegionId | "all"; label: string; n: number }[] = [
    { id: "all", label: t("proxies.region_all"), n: groupNodes.value.length },
  ];
  for (const [id, n] of counts) chips.push({ id, label: t(`proxies.region_${id}`), n });
  return chips;
});
// 切换组 / 节点列表变化后，选中的地区已不存在则回到「全部」。
watch(regionChips, (chips) => {
  if (!chips.some((c) => c.id === region.value)) region.value = "all";
});
// 只在「换了一个组」时重置筛选（按组名比较）；loadGroups 会整体替换对象，不能按引用比较。
watch(() => current.value?.name, () => {
  query.value = "";
  region.value = "all";
});

const tiles = computed(() => {
  const g = current.value;
  if (!g) return [];
  const q = query.value.trim().toLowerCase();
  const list = groupNodes.value
    .filter((n) => (region.value === "all" ? true : regionOf(n) === region.value))
    .filter((n) => (q ? n.toLowerCase().includes(q) : true))
    .map((name) => {
      const d = delayFor(name);
      return {
        name,
        ms: d ?? null,
        timeout: d === null,
        testing: testingNodes.value.has(name),
        selected: g.now === name,
      };
    });
  if (!sortByLatency.value) return list;
  // 已测且成功的升序在前；失败 / 未测排后。
  const key = (x: { ms: number | null; timeout: boolean }) => (x.ms !== null && x.ms > 0 ? x.ms : Infinity);
  return [...list].sort((a, b) => key(a) - key(b));
});

/** 选中节点：带 in-flight 守卫 + 失败提示。 */
const onSelectNode = (name: string) => {
  const g = current.value;
  return g && select.run(() => proxyStore.select(g.name, name));
};

/** 全部测速：对可见组的全部叶子节点去重后批量测速，同时刷新各组自身延迟。 */
function onTestAll() {
  const names = visibleGroups.value.flatMap((g) => nodesOf(g).filter((n) => !proxyStore.groups.some((x) => x.name === n) && n !== "REJECT"));
  void proxyStore.testAll(visibleGroups.value.map((g) => g.name));
  void proxyStore.testNodes(names);
}
const testingAll = computed(() => proxyStore.testing || proxyStore.testingNodes);

// ---- Ctrl+K 聚焦搜索 ----
function onKeydown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
    e.preventDefault();
    searchInput.value?.focus();
    searchInput.value?.select();
  }
}

onMounted(() => {
  void proxyStore.loadGroups();
  window.addEventListener("keydown", onKeydown);
});
onUnmounted(() => window.removeEventListener("keydown", onKeydown));

// 核心从停止 → 运行（含冷启动期间进入本页）后重新加载代理组：
// 否则"启动时进入代理页 → 控制器尚未就绪 → 组列表永久为空"（历史 bug 5）。
watch(
  () => coreStore.status.running,
  (running) => {
    if (running) void proxyStore.loadGroups();
  },
);
</script>

<template>
  <div class="page">
    <div class="proxy-toolbar">
      <h2 class="page-title">{{ $t("proxies.title") }}</h2>
      <label class="search-box">
        <Search :size="16" :stroke-width="1.75" aria-hidden="true" />
        <input
          ref="searchInput"
          v-model="query"
          type="search"
          :placeholder="$t('proxies.search_placeholder')"
          :aria-label="$t('proxies.search_placeholder')"
        />
        <kbd class="kbd" aria-hidden="true">Ctrl K</kbd>
      </label>
      <el-button type="primary" :loading="testingAll" :disabled="visibleGroups.length === 0" @click="onTestAll">
        <Zap :size="16" :stroke-width="1.75" aria-hidden="true" />{{ $t("proxies.test_all") }}
      </el-button>
      <el-button
        class="icon-btn"
        :title="$t('proxies.reload')"
        :aria-label="$t('proxies.reload')"
        :loading="proxyStore.testing"
        @click="proxyStore.loadGroups()"
      >
        <RefreshCw :size="18" :stroke-width="1.75" aria-hidden="true" />
      </el-button>
    </div>

    <CeSegmented
      class="mode-seg"
      :model-value="configStore.proxyMode"
      :options="modeOptions"
      :ariaLabel="$t('general.proxy_mode')"
      @update:model-value="(v) => onModeChange(String(v))"
    />

    <el-empty
      v-if="visibleGroups.length === 0"
      class="proxy-empty"
      :description="
        configStore.proxyMode === 'direct'
          ? $t('proxies.direct_hint')
          : coreStore.status.running
            ? $t('proxies.empty')
            : $t('proxies.core_not_running')
      "
    />

    <div v-else class="split">
      <nav class="group-list" :aria-label="$t('proxies.groups_label')">
        <button
          v-for="g in visibleGroups"
          :key="g.name"
          type="button"
          class="group-row"
          :class="{ 'is-active': g.name === current?.name }"
          :aria-current="g.name === current?.name ? 'true' : undefined"
          @click="selectedName = g.name"
        >
          <span class="group-main">
            <span class="group-name">{{ g.name }}</span>
            <span class="group-type">{{ isAuto(g) ? $t("proxies.type_auto") : $t("proxies.type_manual") }}</span>
          </span>
          <span class="group-now">{{ g.now }}</span>
        </button>
      </nav>

      <section v-if="current" class="ce-card detail" :aria-label="current.name">
        <div class="detail-head">
          <h3 class="detail-title">{{ current.name }}</h3>
          <span class="detail-meta">
            {{ $t("proxies.node_count", { n: groupNodes.length }) }} · {{ $t("proxies.current_node", { name: current.now }) }}
          </span>
          <span class="detail-spacer"></span>
          <el-button text :class="{ 'is-on': sortByLatency }" :aria-pressed="sortByLatency" @click="sortByLatency = !sortByLatency">
            <ArrowDownWideNarrow :size="16" :stroke-width="1.75" aria-hidden="true" />{{ $t("proxies.sort_latency") }}
          </el-button>
        </div>

        <div class="chips" role="group" :aria-label="$t('proxies.region_label')">
          <button
            v-for="c in regionChips"
            :key="c.id"
            type="button"
            class="chip"
            :class="{ 'is-on': region === c.id }"
            :aria-pressed="region === c.id"
            @click="region = c.id"
          >
            {{ c.label }} <span class="chip-n">{{ c.n }}</span>
          </button>
        </div>

        <p v-if="readonlyGroup" class="note">{{ $t("proxies.auto_readonly") }}</p>

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
        <p v-else class="note">{{ $t("proxies.no_match") }}</p>
      </section>
    </div>
  </div>
</template>

<style scoped>
.proxy-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ce-space-3);
  margin-bottom: var(--ce-space-3);
}

.proxy-toolbar .page-title {
  margin: 0 auto 0 0;
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

.search-box input::placeholder {
  color: var(--ce-text-tertiary);
}

.kbd {
  padding: 0 6px;
  border: 1px solid var(--ce-border);
  border-radius: var(--ce-radius-xs);
  background: var(--ce-bg-surface);
  color: var(--ce-text-tertiary);
  font: var(--ce-type-caption);
  font-family: var(--ce-font-mono);
}

.icon-btn {
  width: var(--ce-control-h);
  padding: 0;
}

.mode-seg {
  margin-bottom: var(--ce-space-5);
}

/* ---- 主从双栏 ---- */
.split {
  display: grid;
  grid-template-columns: 280px minmax(0, 1fr);
  gap: var(--ce-space-4);
  align-items: start;
}

.group-list {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--ce-border);
  border-radius: var(--ce-radius-lg);
  background: var(--ce-bg-surface);
}

.group-row {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-height: 60px;
  padding: var(--ce-space-2) var(--ce-space-4);
  border: 0;
  border-top: 1px solid var(--ce-divider);
  background: transparent;
  color: var(--ce-text-primary);
  text-align: left;
  cursor: pointer;
}

.group-row:first-child {
  border-top: 0;
}

.group-row:hover {
  background: var(--ce-fill-hover);
}

.group-row.is-active {
  background: var(--ce-accent-soft);
}

.group-row.is-active .group-name {
  color: var(--ce-accent-fg);
}

.group-row:focus-visible {
  outline: 2px solid var(--ce-accent-bg);
  outline-offset: -2px;
}

.group-main {
  display: flex;
  align-items: center;
  gap: var(--ce-space-2);
}

.group-name {
  font: var(--ce-type-body);
  font-weight: 500;
}

.group-type {
  padding: 0 6px;
  border-radius: var(--ce-radius-xs);
  background: var(--ce-fill-soft);
  color: var(--ce-text-secondary);
  font: var(--ce-type-caption);
}

.group-now {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--ce-text-tertiary);
  font: var(--ce-type-callout);
}

/* ---- 右栏 ---- */
.detail {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-4);
  min-width: 0;
}

.detail-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ce-space-3);
}

.detail-title {
  margin: 0;
  font: var(--ce-type-title-2);
}

.detail-meta {
  color: var(--ce-text-tertiary);
  font: var(--ce-type-callout);
  font-variant-numeric: tabular-nums;
}

.detail-spacer {
  flex: 1;
}

.detail-head .is-on {
  background: var(--ce-accent-soft);
}

.chips {
  display: flex;
  flex-wrap: wrap;
  gap: var(--ce-space-2);
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

.chip:focus-visible {
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

.note {
  margin: 0;
  color: var(--ce-text-secondary);
  font: var(--ce-type-callout);
}

.tile-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: var(--ce-space-3);
}

/* 窗口 < 960：左栏改为顶部横向芯片。 */
@media (max-width: 959px) {
  .split {
    grid-template-columns: minmax(0, 1fr);
  }

  .group-list {
    flex-direction: row;
    gap: var(--ce-space-2);
    overflow-x: auto;
    border: 0;
    border-radius: 0;
    background: transparent;
  }

  .group-row {
    flex: none;
    min-height: var(--ce-control-h);
    padding: 0 var(--ce-space-4);
    border: 1px solid var(--ce-border);
    border-radius: var(--ce-radius-full);
    background: var(--ce-bg-surface);
  }

  .group-row:first-child {
    border-top: 1px solid var(--ce-border);
  }

  .group-row.is-active {
    border-color: transparent;
    background: var(--ce-accent-soft);
  }

  .group-now {
    display: none;
  }
}
</style>
