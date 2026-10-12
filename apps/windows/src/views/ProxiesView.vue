<!-- src/views/ProxiesView.vue - 代理：策略组列表 + 节点格（主从双栏）
     - 左栏单一平铺列表（扶梯出行→人工智能→影音视听→自动优选→人工优选）
     - 右栏：选中组的节点格 + 搜索
     - 自动优选（URLTest）组节点格只读：由测速自动选择 -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowDownUp, Search, Zap } from "lucide-vue-next";
import type { ProxyGroup } from "@/api/proxy";
import CeNodeTile from "@/components/ui/CeNodeTile.vue";
import CeSegmented from "@/components/ui/CeSegmented.vue";
import { useAction } from "@/composables/useAction";
import { useNodeRetest } from "@/composables/useNodeRetest";
import { resolveGroupId, sortRuleGroups } from "@/constants/groups";
import { useConfigStore } from "@/stores/config";
import { useCoreStore } from "@/stores/core";
import { useProxyStore } from "@/stores/proxy";

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

const selectedName = ref<string | null>(null);
/** 默认落地人工优选（可交互的真实节点格）；无该组时回退列表首组。 */
const current = computed(
  () =>
    visibleGroups.value.find((g) => g.name === selectedName.value) ??
    visibleGroups.value.find((g) => resolveGroupId(g.name) === "manual") ??
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

// ---- 搜索 ----
const query = ref("");
const searchInput = ref<HTMLInputElement | null>(null);

const groupNodes = computed(() => (current.value ? nodesOf(current.value) : []));
watch(
  () => current.value?.name,
  () => {
    query.value = "";
  },
);

// ---- 排序：延迟（默认，已测升序在前）/ 名称 ----
const sortMode = ref<"delay" | "name">("delay");
const sortTitle = computed(() => t(sortMode.value === "delay" ? "proxies.sort_latency" : "proxies.sort_name"));
function toggleSort() {
  sortMode.value = sortMode.value === "delay" ? "name" : "delay";
}

const tiles = computed(() => {
  const g = current.value;
  if (!g) return [];
  const q = query.value.trim().toLowerCase();
  return groupNodes.value
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
      if (sortMode.value === "name") return a.name.localeCompare(b.name);
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

// ---- 首次启动：人工优选默认选中延迟最低节点（一次性） ----
// 规格：人工优选默认=全网延迟最低节点、可手动改。仅首次导入/启动后自动跑一次
// 全量测速并选到最低延迟者；完成后打持久化标记，此后永不覆盖用户手动选择。
const MANUAL_DEFAULT_KEY = "ce:manual-default:v1";
let manualDefaultRunning = false;

async function initManualDefault() {
  if (manualDefaultRunning) return;
  if (localStorage.getItem(MANUAL_DEFAULT_KEY)) return;
  if (!coreStore.status.running) return;
  const manual = proxyStore.groups.find((g) => resolveGroupId(g.name) === "manual");
  const manualNodes = manual ? manual.all.filter((n) => n !== "DIRECT") : [];
  if (!manual || manualNodes.length === 0) return;
  manualDefaultRunning = true;
  const before = manual.now;
  try {
    const names = [
      ...new Set(
        visibleGroups.value
          .filter((g) => ["manual", "auto"].includes(resolveGroupId(g.name)))
          .flatMap((g) => nodesOf(g)),
      ),
    ];
    await proxyStore.testNodes(names);
    await proxyStore.loadGroups();
    const fresh = proxyStore.groups.find((g) => resolveGroupId(g.name) === "manual");
    // 测速期间用户手动改过 → 尊重用户选择，不覆盖
    if (!fresh || fresh.now !== before) return;
    let best: string | null = null;
    let bestMs = Infinity;
    for (const n of manualNodes) {
      const d = proxyStore.nodeDelays[n];
      if (typeof d === "number" && d > 0 && d < bestMs) {
        bestMs = d;
        best = n;
      }
    }
    if (best && best !== fresh.now) await proxyStore.select(fresh.name, best);
  } catch {
    // 一次性初始化失败静默；无论成败均打标记，避免每次启动重复全量测速
  } finally {
    localStorage.setItem(MANUAL_DEFAULT_KEY, "1");
    manualDefaultRunning = false;
  }
}

onMounted(() => {
  void proxyStore.loadGroups().then(() => initManualDefault());
  window.addEventListener("keydown", onKeydown);
});
onUnmounted(() => window.removeEventListener("keydown", onKeydown));

watch(
  () => coreStore.status.running,
  (running) => {
    if (running) void proxyStore.loadGroups().then(() => initManualDefault());
  },
);
</script>

<template>
  <div class="page">
    <div class="proxy-toolbar">
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
        <button
          v-for="g in visibleGroups"
          :key="g.name"
          type="button"
          class="group-row"
          :class="{ 'is-active': g.name === current?.name }"
          :aria-current="g.name === current?.name ? 'true' : undefined"
          @click="selectedName = g.name"
        >
          <span class="group-name">{{ g.name }}</span>
          <!-- 当前节点同行右侧：不设 max-width 吃满剩余栏宽，超长省略号 + title 悬停看全名 -->
          <span v-if="g.now" class="group-now" :title="g.now">{{ g.now }}</span>
        </button>
      </nav>

      <section v-if="current" class="ce-card detail" :aria-label="current.name">
        <div class="detail-head">
          <h3 class="detail-title">{{ current.name }}</h3>
          <span class="detail-meta">{{ $t("proxies.node_count", { n: groupNodes.length }) }}</span>
          <span class="detail-actions">
            <el-button
              class="icon-btn"
              :title="sortTitle"
              :aria-label="sortTitle"
              @click="toggleSort"
            >
              <ArrowDownUp :size="18" :stroke-width="1.75" aria-hidden="true" />
            </el-button>
            <el-button type="primary" :loading="testingAll" @click="onTestAll">
              <Zap :size="16" :stroke-width="1.75" aria-hidden="true" />{{ $t("proxies.test_all") }}
            </el-button>
          </span>
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
  margin-bottom: var(--ce-space-5);
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
}

.group-list {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-1);
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

.group-name {
  flex: none;
  font: var(--ce-type-body-strong);
  color: inherit;
  white-space: nowrap;
}

.group-now {
  flex: 1;
  min-width: 0;
  text-align: right;
  font: var(--ce-type-callout);
  color: var(--ce-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  /* 不设 max-width：吃满组行剩余宽度，长节点名以省略号自适应收尾 */
}

.is-active .group-now {
  color: inherit;
}

.detail {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-3);
  min-width: 0;
}

.detail-head {
  display: flex;
  align-items: center;
  gap: var(--ce-space-3);
}

.detail-actions {
  display: flex;
  align-items: center;
  gap: var(--ce-space-2);
  margin-left: auto;
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

.note {
  margin: 0;
  font: var(--ce-type-callout);
  color: var(--ce-text-tertiary);
}

.tile-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: var(--ce-space-3);
}

.icon-btn {
  width: var(--ce-control-h);
  padding: 0;
}
</style>
