<!-- src/views/ProxiesView.vue - 代理：策略组列表 + 节点格（主从双栏）
     - 左栏分两段：「选择节点」（人工优选/自动优选）+「分流规则」（扶梯出行等）
     - 右栏：选中组的节点格 + 搜索 + 地区筛选（节点多时才显示）
     - 自动优选（URLTest）组节点格只读：由测速自动选择 -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { RefreshCw, Search, Zap } from "lucide-vue-next";
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

const select = useAction();
const modeAction = useAction();
const { testing: testingNodes, retest: onRetest } = useNodeRetest();

const modeOptions = computed(() =>
  ["rule", "global", "direct"].map((m) => ({ value: m, label: t(`tray.mode_${m}`) })),
);
const onModeChange = (mode: string) => modeAction.run(() => configStore.setProxyMode(mode));

const visibleGroups = computed<ProxyGroup[]>(() => {
  const mode = configStore.proxyMode;
  if (mode === "global") return proxyStore.groups.filter((g) => resolveGroupId(g.name) === "global");
  if (mode === "direct") return [];
  return sortRuleGroups(proxyStore.groups);
});

/** 左栏两段：叶子组（选节点）→ 路由组（分流规则） */
const leafGroups = computed(() =>
  visibleGroups.value.filter((g) => ["manual", "auto"].includes(resolveGroupId(g.name))),
);
const routeGroups = computed(() =>
  visibleGroups.value.filter((g) => !["manual", "auto"].includes(resolveGroupId(g.name))),
);

const selectedName = ref<string | null>(null);
const current = computed(
  () =>
    visibleGroups.value.find((g) => g.name === selectedName.value) ??
    leafGroups.value[0] ??
    visibleGroups.value[0],
);
const isAuto = (g: ProxyGroup) => resolveGroupId(g.name) === "auto" || g.type === "URLTest";
const isLeaf = (g: ProxyGroup) => ["manual", "auto"].includes(resolveGroupId(g.name));
const readonlyGroup = computed(() => (current.value ? isAuto(current.value) : false));

function nodesOf(g: ProxyGroup): string[] {
  return isLeaf(g) ? g.all.filter((p) => p !== "DIRECT") : g.all;
}

function delayFor(name: string): number | null | undefined {
  const d = proxyStore.nodeDelays[name];
  if (d !== undefined) return d;
  return name in proxyStore.delays ? proxyStore.delays[name] : undefined;
}

// ---- 搜索 / 地区 ----
const query = ref("");
const region = ref<RegionId | "all">("all");
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
watch(regionChips, (chips) => {
  if (!chips.some((c) => c.id === region.value)) region.value = "all";
});
watch(
  () => current.value?.name,
  () => {
    query.value = "";
    region.value = "all";
  },
);

const tiles = computed(() => {
  const g = current.value;
  if (!g) return [];
  const q = query.value.trim().toLowerCase();
  return groupNodes.value
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
    })
    .sort((a, b) => {
      const ka = a.ms !== null && a.ms > 0 ? a.ms : Infinity;
      const kb = b.ms !== null && b.ms > 0 ? b.ms : Infinity;
      return ka - kb;
    });
});

/** 选中节点：带 in-flight 守卫 + 失败提示。 */
const onSelectNode = (name: string) => {
  const g = current.value;
  return g && select.run(() => proxyStore.select(g.name, name));
};

function onTestAll() {
  const names = visibleGroups.value
    .flatMap((g) => nodesOf(g).filter((n) => !proxyStore.groups.some((x) => x.name === n) && n !== "REJECT"));
  void proxyStore.testAll(visibleGroups.value.map((g) => g.name));
  void proxyStore.testNodes(names);
}
const testingAll = computed(() => proxyStore.testing || proxyStore.testingNodes);

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
      <CeSegmented
        :model-value="configStore.proxyMode"
        :options="modeOptions"
        :ariaLabel="$t('general.proxy_mode')"
        @update:model-value="(v) => onModeChange(String(v))"
      />
      <span class="toolbar-spacer"></span>
      <label class="search-box">
        <Search :size="16" :stroke-width="1.75" aria-hidden="true" />
        <input
          ref="searchInput"
          v-model="query"
          type="search"
          :placeholder="$t('proxies.search_placeholder')"
          :aria-label="$t('proxies.search_placeholder')"
        />
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
        <template v-if="leafGroups.length">
          <span class="group-section">{{ $t("proxies.section_pick") }}</span>
          <button
            v-for="g in leafGroups"
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
        </template>
        <template v-if="routeGroups.length">
          <span class="group-section">{{ $t("proxies.section_route") }}</span>
          <button
            v-for="g in routeGroups"
            :key="g.name"
            type="button"
            class="group-row"
            :class="{ 'is-active': g.name === current?.name }"
            :aria-current="g.name === current?.name ? 'true' : undefined"
            @click="selectedName = g.name"
          >
            <span class="group-main">
              <span class="group-name">{{ g.name }}</span>
            </span>
            <span class="group-now">{{ g.now }}</span>
          </button>
        </template>
      </nav>

      <section v-if="current" class="ce-card detail" :aria-label="current.name">
        <div class="detail-head">
          <h3 class="detail-title">{{ current.name }}</h3>
          <span class="detail-meta">{{ $t("proxies.node_count", { n: groupNodes.length }) }}</span>
        </div>

        <div
          v-if="regionChips.length > 2 && groupNodes.length > 8"
          class="chips"
          role="group"
          :aria-label="$t('proxies.region_label')"
        >
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
  margin: 0;
}

.toolbar-spacer {
  flex: 1;
}

.search-box {
  display: flex;
  align-items: center;
  gap: var(--ce-space-2);
  width: 220px;
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
}

.search-box input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  color: var(--ce-text-primary);
  font: var(--ce-type-body);
}

.search-box input::placeholder {
  color: var(--ce-text-tertiary);
}

.proxy-empty {
  margin-top: var(--ce-space-8);
}

.split {
  display: grid;
  grid-template-columns: 240px minmax(0, 1fr);
  gap: var(--ce-space-4);
  min-height: 0;
}

@media (max-width: 959px) {
  .split {
    grid-template-columns: minmax(0, 1fr);
  }
  .group-list {
    display: flex;
    overflow-x: auto;
    gap: var(--ce-space-2);
    padding-bottom: var(--ce-space-2);
  }
  .group-section {
    display: none;
  }
}

.group-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.group-section {
  font: var(--ce-type-caption);
  color: var(--ce-text-tertiary);
  padding: var(--ce-space-2) var(--ce-space-3) var(--ce-space-1);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.group-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ce-space-2);
  padding: var(--ce-space-2) var(--ce-space-3);
  border: none;
  border-radius: var(--ce-radius-sm);
  background: transparent;
  cursor: pointer;
  text-align: left;
  min-height: var(--ce-row-h);
  transition: background-color var(--ce-dur-fast) var(--ce-ease-standard);
}

.group-row:hover {
  background: var(--ce-fill-hover);
}

.group-row.is-active {
  background: var(--ce-accent-soft);
  color: var(--ce-accent-fg);
}

.group-main {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.group-name {
  font: var(--ce-type-body-strong);
  color: inherit;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.group-type {
  font: var(--ce-type-caption);
  color: var(--ce-text-tertiary);
}

.is-active .group-type {
  color: inherit;
}

.group-now {
  font: var(--ce-type-callout);
  color: var(--ce-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 120px;
}

.is-active .group-now {
  color: inherit;
}

.detail {
  padding: var(--ce-space-4);
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-3);
  min-width: 0;
}

.detail-head {
  display: flex;
  align-items: baseline;
  gap: var(--ce-space-3);
}

.detail-title {
  margin: 0;
  font: var(--ce-type-title-3);
  color: var(--ce-text-primary);
}

.detail-meta {
  font: var(--ce-type-callout);
  color: var(--ce-text-tertiary);
}

.chips {
  display: flex;
  flex-wrap: wrap;
  gap: var(--ce-space-2);
}

.chip {
  height: 28px;
  padding: 0 var(--ce-space-3);
  border: 1px solid var(--ce-border);
  border-radius: var(--ce-radius-full);
  background: var(--ce-bg-surface);
  color: var(--ce-text-secondary);
  font: var(--ce-type-caption);
  cursor: pointer;
  transition: all var(--ce-dur-fast) var(--ce-ease-standard);
}

.chip:hover {
  border-color: var(--ce-text-tertiary);
}

.chip.is-on {
  background: var(--ce-accent-soft);
  border-color: var(--ce-accent-bg);
  color: var(--ce-accent-fg);
}

.chip-n {
  color: var(--ce-text-tertiary);
  font-variant-numeric: tabular-nums;
}

.chip.is-on .chip-n {
  color: inherit;
}

.note {
  margin: 0;
  font: var(--ce-type-callout);
  color: var(--ce-text-tertiary);
}

.tile-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: var(--ce-space-2);
}

.icon-btn {
  padding: 0 var(--ce-space-2);
}
</style>
