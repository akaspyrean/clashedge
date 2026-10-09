<!-- src/views/RulesView.vue - 规则（只读）：当前运行配置里生效的规则列表。
     数据来自 get_rules（Rust 侧只保留 type / payload / proxy）。
     - 网格行（不用 <table> 套循环），行高 44；超过 PAGE_SIZE 条时分页，不一次渲染全部
     - 策略颜色：DIRECT 次要、REJECT 危险、其余 accent；命中数接口不提供，故不显示该列
     - 规则集视图暂无数据来源，分段控件隐藏 -->
<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { Search } from "lucide-vue-next";
import { rulesApi, type RuleInfo } from "@/api/rules";
import { useNotify } from "@/composables/useNotify";
import { useCoreStore } from "@/stores/core";

const PAGE_SIZE = 500;

const { t } = useI18n();
const core = useCoreStore();
const notify = useNotify();

const rules = ref<RuleInfo[]>([]);
const loading = ref(false);
const failed = ref(false);
const query = ref("");
const page = ref(1);

async function load() {
  if (!core.status.running) {
    rules.value = [];
    return;
  }
  loading.value = true;
  failed.value = false;
  try {
    rules.value = (await rulesApi.list()) ?? [];
  } catch (e) {
    rules.value = [];
    failed.value = true;
    notify.fail(e, `${t("rules.load_failed")}：`);
  } finally {
    loading.value = false;
  }
}

onMounted(load);
// 核心从停止 → 运行（含冷启动期间进入本页）后重新加载；停止后清空。
watch(() => core.status.running, load);

/** 带原始序号的行，过滤后序号仍指向规则在配置里的真实位置。 */
const indexed = computed(() => rules.value.map((r, i) => ({ ...r, n: i + 1 })));
const filtered = computed(() => {
  const q = query.value.trim().toLowerCase();
  if (!q) return indexed.value;
  return indexed.value.filter(
    (r) =>
      r.type.toLowerCase().includes(q) || r.payload.toLowerCase().includes(q) || r.proxy.toLowerCase().includes(q),
  );
});

const pageCount = computed(() => Math.max(1, Math.ceil(filtered.value.length / PAGE_SIZE)));
watch([query, () => rules.value.length], () => (page.value = 1));
const shown = computed(() => filtered.value.slice((page.value - 1) * PAGE_SIZE, page.value * PAGE_SIZE));
const range = computed(() => ({
  from: filtered.value.length === 0 ? 0 : (page.value - 1) * PAGE_SIZE + 1,
  to: Math.min(page.value * PAGE_SIZE, filtered.value.length),
  total: filtered.value.length,
}));

function policyClass(proxy: string): string {
  if (proxy === "DIRECT") return "is-direct";
  if (proxy.startsWith("REJECT")) return "is-reject";
  return "is-proxy";
}
</script>

<template>
  <div class="page">
    <div class="toolbar">
      <h2 class="page-title">{{ $t("rules.title") }}</h2>
      <span v-if="rules.length" class="count">{{ $t("rules.count", { n: rules.length }) }}</span>
      <label class="search-box">
        <Search :size="16" :stroke-width="1.75" aria-hidden="true" />
        <input
          v-model="query"
          type="search"
          :placeholder="$t('rules.search_placeholder')"
          :aria-label="$t('rules.search_placeholder')"
        />
      </label>
    </div>

    <el-empty
      v-if="!core.status.running"
      class="rules-empty"
      :description="$t('rules.core_not_running')"
    >
      <p class="hint">{{ $t("rules.core_not_running_hint") }}</p>
    </el-empty>
    <el-empty v-else-if="failed" class="rules-empty" :description="$t('rules.load_failed')" />
    <el-empty
      v-else-if="!loading && rules.length === 0"
      class="rules-empty"
      :description="$t('rules.empty')"
    />
    <el-empty v-else-if="filtered.length === 0 && !loading" class="rules-empty" :description="$t('rules.no_match')" />

    <section v-else class="ce-card rules-card" :aria-label="$t('rules.list_label')" :aria-busy="loading">
      <div class="row head" role="row">
        <span>{{ $t("rules.col_index") }}</span>
        <span>{{ $t("rules.col_type") }}</span>
        <span>{{ $t("rules.col_payload") }}</span>
        <span>{{ $t("rules.col_policy") }}</span>
      </div>
      <ul class="rule-list">
        <li v-for="r in shown" :key="r.n" class="row">
          <span class="idx">{{ r.n }}</span>
          <span><span class="type-tag">{{ r.type }}</span></span>
          <span class="payload" :title="r.payload">{{ r.payload || "—" }}</span>
          <span class="policy" :class="policyClass(r.proxy)">{{ r.proxy }}</span>
        </li>
      </ul>
      <div v-if="pageCount > 1" class="pager">
        <span class="range">{{ $t("rules.page_range", range) }}</span>
        <span class="pager-actions">
          <el-button size="small" :disabled="page <= 1" @click="page--">{{ $t("rules.page_prev") }}</el-button>
          <span class="page-no">{{ page }} / {{ pageCount }}</span>
          <el-button size="small" :disabled="page >= pageCount" @click="page++">{{ $t("rules.page_next") }}</el-button>
        </span>
      </div>
    </section>
  </div>
</template>

<style scoped>
.toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ce-space-3);
  margin-bottom: var(--ce-space-5);
}

.toolbar .page-title {
  margin: 0;
}

.count {
  margin-right: auto;
  color: var(--ce-text-tertiary);
  font: var(--ce-type-callout);
  font-variant-numeric: tabular-nums;
}

.search-box {
  display: flex;
  align-items: center;
  gap: var(--ce-space-2);
  width: 300px;
  height: var(--ce-control-h);
  margin-left: auto;
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

.hint {
  margin: 0;
  color: var(--ce-text-tertiary);
  font: var(--ce-type-callout);
}

.rules-card {
  padding: 0;
  overflow: hidden;
}

.row {
  display: grid;
  grid-template-columns: 64px 160px minmax(0, 1fr) 160px;
  align-items: center;
  gap: var(--ce-space-3);
  min-height: var(--ce-row-h);
  padding: 0 var(--ce-space-4);
}

.head {
  min-height: 36px;
  border-bottom: 1px solid var(--ce-divider);
  color: var(--ce-text-tertiary);
  font: var(--ce-type-caption);
}

.rule-list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.rule-list .row {
  border-top: 1px solid var(--ce-divider);
}

.rule-list .row:first-child {
  border-top: 0;
}

.rule-list .row:hover {
  background: var(--ce-fill-hover);
}

.idx {
  color: var(--ce-text-tertiary);
  font: var(--ce-type-mono);
  font-variant-numeric: tabular-nums;
}

.type-tag {
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

.payload {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font: var(--ce-type-mono);
}

.policy {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font: var(--ce-type-body);
  font-weight: 500;
}

.policy.is-direct {
  color: var(--ce-text-secondary);
}

.policy.is-reject {
  color: var(--ce-danger-fg);
}

.policy.is-proxy {
  color: var(--ce-accent-fg);
}

.pager {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--ce-space-3);
  padding: var(--ce-space-3) var(--ce-space-4);
  border-top: 1px solid var(--ce-divider);
  color: var(--ce-text-secondary);
  font: var(--ce-type-callout);
  font-variant-numeric: tabular-nums;
}

.pager-actions {
  display: flex;
  align-items: center;
  gap: var(--ce-space-2);
}

@media (max-width: 749px) {
  .row {
    grid-template-columns: 48px minmax(0, 1fr) 120px;
  }

  .row > :nth-child(2) {
    display: none;
  }
}
</style>
