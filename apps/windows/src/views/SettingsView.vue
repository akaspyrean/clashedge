<!-- src/views/SettingsView.vue - 设置：单页分组（常规 / 代理 / TUN / 高级 / 关于）+ 搜索
     本文件是外壳：加载配置、统一的「保存」、设置搜索（高亮命中并滚动到位）。
     每个分组的内容仍是独立组件（components/settings/*），各自管理状态与动作，
     所有设置项都在，只是由 5 个 Tab 改成了同一页的分组卡片；宽屏分两列。
     配置降级（config.yaml 损坏）的提示与确认在应用顶部横幅（DegradedBanner），不在本页。 -->
<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { Search } from "lucide-vue-next";
import AboutTab from "@/components/settings/AboutTab.vue";
import AdvancedTab from "@/components/settings/AdvancedTab.vue";
import GeneralTab from "@/components/settings/GeneralTab.vue";
import ProxyTab from "@/components/settings/ProxyTab.vue";
import TunTab from "@/components/settings/TunTab.vue";
import { useSettingsSave } from "@/composables/useSettingsSave";
import { useAppStore } from "@/stores/app";
import { useConfigStore } from "@/stores/config";

const appStore = useAppStore();
const configStore = useConfigStore();
const { saving, save } = useSettingsSave();

const pageEl = ref<HTMLElement | null>(null);

onMounted(async () => {
  if (!configStore.config) await configStore.load();
  await appStore.loadAutostart();
});

// 分组：顺序即页面顺序；宽屏左列 常规 + 代理，右列 TUN + 高级 + 关于。
const left = [
  { id: "general", title: "settings.tabs.general", comp: GeneralTab },
  { id: "proxy", title: "settings.tabs.proxy", comp: ProxyTab },
];
const right = [
  { id: "tun", title: "settings.tabs.tun", comp: TunTab },
  { id: "advanced", title: "settings.tabs.advanced", comp: AdvancedTab },
  { id: "about", title: "settings.tabs.about", comp: AboutTab },
];

// ---- 搜索：命中的偏好行加 .pref-hit 高亮，并滚动到第一个命中项 ----
const query = ref("");
const hitCount = ref(0);
const searched = computed(() => query.value.trim().length > 0);

async function applySearch() {
  await nextTick();
  const root = pageEl.value;
  if (!root) return;
  const q = query.value.trim().toLowerCase();
  const rows = Array.from(root.querySelectorAll<HTMLElement>(".pref-row"));
  let first: HTMLElement | null = null;
  let n = 0;
  for (const row of rows) {
    const hit = q !== "" && (row.textContent ?? "").toLowerCase().includes(q);
    row.classList.toggle("pref-hit", hit);
    if (hit) {
      n++;
      first ??= row;
    }
  }
  hitCount.value = n;
  first?.scrollIntoView?.({ block: "center", behavior: "smooth" });
}
watch(query, applySearch);
</script>

<template>
  <div v-if="configStore.config" ref="pageEl" class="page settings-page">
    <div class="toolbar">
      <h2 class="page-title">{{ $t("settings.title") }}</h2>
      <span v-if="searched" class="found" role="status">
        {{ hitCount > 0 ? $t("settings.search_count", { n: hitCount }) : $t("settings.search_none") }}
      </span>
      <label class="search-box">
        <Search :size="16" :stroke-width="1.75" aria-hidden="true" />
        <input
          v-model="query"
          type="search"
          :placeholder="$t('settings.search_placeholder')"
          :aria-label="$t('settings.search_placeholder')"
        />
      </label>
      <el-button type="primary" :loading="saving" @click="save">{{ $t("common.save") }}</el-button>
    </div>

    <div class="columns">
      <div class="column">
        <section v-for="g in left" :id="`settings-${g.id}`" :key="g.id" class="ce-card group" :aria-labelledby="`settings-${g.id}-title`">
          <h3 :id="`settings-${g.id}-title`" class="group-title">{{ $t(g.title) }}</h3>
          <component :is="g.comp" />
        </section>
      </div>
      <div class="column">
        <section v-for="g in right" :id="`settings-${g.id}`" :key="g.id" class="ce-card group" :aria-labelledby="`settings-${g.id}-title`">
          <h3 :id="`settings-${g.id}-title`" class="group-title">{{ $t(g.title) }}</h3>
          <component :is="g.comp" />
        </section>
      </div>
    </div>
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
  margin: 0 auto 0 0;
}

.found {
  color: var(--ce-text-tertiary);
  font: var(--ce-type-callout);
  font-variant-numeric: tabular-nums;
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

/* 宽屏两列；< 960 单列。 */
.columns {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--ce-space-4);
  align-items: start;
}

@media (max-width: 959px) {
  .columns {
    grid-template-columns: minmax(0, 1fr);
  }
}

.column {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-4);
  min-width: 0;
}

.group-title {
  margin: 0 0 var(--ce-space-2);
  font: var(--ce-type-title-3);
  color: var(--ce-text-primary);
}
</style>
