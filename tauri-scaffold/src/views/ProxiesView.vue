<!-- src/views/ProxiesView.vue - 代理组：切换代理模式、查看各组节点、手动选择与延迟测试 -->
<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { Lightning, Refresh } from "@element-plus/icons-vue";
import { ElMessage } from "element-plus";
import { proxyApi, type ProxyGroup } from "@/api/proxy";
import { resolveGroupId } from "@/constants/groups";
import { useConfigStore } from "@/stores/config";
import { useCoreStore } from "@/stores/core";
import { useProxyStore } from "@/stores/proxy";

const proxyStore = useProxyStore();
const configStore = useConfigStore();
const coreStore = useCoreStore();

// 节点选中 in-flight 守卫：连点节点时只允许一个 select 在途，避免乱序覆盖。
const selecting = ref(false);
// 单组测速（testOne）在途集合：组粒度守卫，避免同组连点重复请求。
const testingGroups = ref(new Set<string>());

// mihomo 官方模板仅这三值；script 是 Clash Premium 遗留，后端会拒绝。
const proxyModes = ["rule", "global", "direct"];

/** 切换全局代理模式：走统一编排层（持久化 + 实时 PATCH 核心 + 托盘刷新），
 *  成功后把本地 store 同步为真实状态。 */
async function onModeChange(mode: string) {
  try {
    await proxyApi.setProxyMode(mode);
    if (configStore.config) configStore.config.mode = mode;
  } catch (e) {
    ElMessage.error(String(e));
  }
}

/** 选中节点：带 in-flight 守卫 + 失败提示。 */
async function onSelectNode(group: string, proxy: string) {
  if (selecting.value) return;
  selecting.value = true;
  try {
    await proxyStore.select(group, proxy);
  } catch (e) {
    ElMessage.error(String(e));
  } finally {
    selecting.value = false;
  }
}

/** 单组延迟测试：组粒度 in-flight 守卫，连点不重复发起。 */
async function onTestGroup(group: string) {
  if (testingGroups.value.has(group)) return;
  testingGroups.value.add(group);
  try {
    await proxyStore.testOne(group);
  } catch (e) {
    ElMessage.error(String(e));
  } finally {
    testingGroups.value.delete(group);
  }
}

/** 手动测速（组内逐节点）：复用 store 的全局 testingNodes 标志防重入。 */
async function onManualTest(group: string) {
  if (proxyStore.testingNodes) return;
  await proxyStore.testGroupProxies(group);
}

/** 组延迟文本：未测试 / 失败时显示 "—"。 */
function delayOf(name: string): string {
  const d = proxyStore.delays[name];
  return d == null ? "—" : `${d} ms`;
}

/** 人工优选组（手动挑选节点场景）专属：提供组内节点逐一测速。 */
const isManual = (g: ProxyGroup) => resolveGroupId(g.name) === "manual";

/** 节点延迟文本：未测不显示；测过失败显示 "—"；成功显示 "N ms"。 */
function nodeDelayOf(name: string): string {
  const d = proxyStore.nodeDelays[name];
  if (d === undefined) return "";
  return d === null ? "—" : `${d} ms`;
}

/** 规则模式下代理组的规范排序（自上而下）；mihomo /proxies 返回无序，需显式排序。
 *  排序按语义 ID 比较，不依赖中文字面量；自定义组 resolveGroupId 回退原名，排最后。 */
const GROUP_ORDER = ["proxy", "ai", "media", "manual", "auto"];

/** 按当前模式筛选可见组并排序：rule 显示 5 组（隐藏 GLOBAL）、global 只显示 GLOBAL、direct 无组。 */
const visibleGroups = computed(() => {
  const mode = configStore.proxyMode;
  if (mode === "global")
    return proxyStore.groups.filter((g) => resolveGroupId(g.name) === "global");
  if (mode === "direct") return [];
  const rank = new Map(GROUP_ORDER.map((id, i) => [id, i]));
  return proxyStore.groups
    .filter((g) => resolveGroupId(g.name) !== "global")
    .sort(
      (a, b) =>
        (rank.get(resolveGroupId(a.name)) ?? 99) -
        (rank.get(resolveGroupId(b.name)) ?? 99)
    );
});

/** 叶子组（人工优选/自动优选）屏蔽内置 DIRECT，只显示真实节点。 */
function visibleProxies(g: ProxyGroup): string[] {
  const id = resolveGroupId(g.name);
  return id === "manual" || id === "auto"
    ? g.all.filter((p) => p !== "DIRECT")
    : g.all;
}

onMounted(() => {
  void proxyStore.loadGroups();
});

// 核心从停止 → 运行（含冷启动期间进入本页）后重新加载代理组：
// 否则"启动时进入代理页 → 控制器尚未就绪 → 组列表永久为空"（历史 bug 5）。
watch(
  () => coreStore.status.running,
  (running) => {
    if (running) void proxyStore.loadGroups();
  }
);
</script>

<template>
  <div class="page">
    <h2 class="page-title">{{ $t("proxies.title") }}</h2>

    <div class="proxy-toolbar">
      <div class="mode-field">
        <span class="mode-label">{{ $t("general.proxy_mode") }}</span>
        <el-select
          :model-value="configStore.proxyMode"
          style="width: 200px"
          @change="onModeChange"
        >
          <el-option
            v-for="m in proxyModes"
            :key="m"
            :label="$t('tray.mode_' + m)"
            :value="m"
          />
        </el-select>
      </div>
      <el-button
        type="primary"
        :loading="proxyStore.testing"
        :disabled="visibleGroups.length === 0"
        @click="proxyStore.testAll(visibleGroups.map((g) => g.name))"
      >
        {{ $t("proxies.test_all") }}
      </el-button>
      <el-button
        text
        :title="$t('proxies.reload')"
        :loading="proxyStore.testing"
        @click="proxyStore.loadGroups()"
      >
        <el-icon><Refresh /></el-icon>
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

    <el-collapse v-else>
      <el-collapse-item v-for="g in visibleGroups" :key="g.name" :name="g.name">
        <template #title>
          <div class="group-title">
            <span class="group-name">{{ g.name }}</span>
            <el-tag size="small" effect="plain">{{ g.type }}</el-tag>
            <span class="group-now">{{ g.now }}</span>
            <el-tag size="small" type="info" effect="plain">
              {{ $t("proxies.latency") }} {{ delayOf(g.name) }}
            </el-tag>
            <span class="group-actions">
              <el-button
                v-if="isManual(g)"
                size="small"
                circle
                text
                :loading="proxyStore.testingNodes"
                :title="$t('proxies.manual_test')"
                @click.stop="onManualTest(g.name)"
              >
                <el-icon><Lightning /></el-icon>
              </el-button>
              <el-button
                size="small"
                circle
                text
                :loading="testingGroups.has(g.name)"
                :title="$t('proxies.latency')"
                @click.stop="onTestGroup(g.name)"
              >
                <el-icon><Refresh /></el-icon>
              </el-button>
            </span>
          </div>
        </template>

        <div class="proxy-list">
          <button
            v-for="proxy in visibleProxies(g)"
            :key="proxy"
            type="button"
            class="proxy-item"
            :class="{ active: proxy === g.now }"
            @click="onSelectNode(g.name, proxy)"
          >
            <span class="proxy-node">{{ proxy }}</span>
            <span v-if="isManual(g)" class="proxy-node-delay">
              {{ nodeDelayOf(proxy) }}
            </span>
          </button>
        </div>
      </el-collapse-item>
    </el-collapse>
  </div>
</template>

<style scoped>
.proxy-toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-bottom: var(--space-4);
}

.mode-field {
  display: flex;
  align-items: center;
  gap: 8px;
}

.mode-label {
  font-size: 13px;
  color: var(--text-tertiary);
  white-space: nowrap;
}

.group-title {
  display: flex;
  align-items: center;
  gap: 10px;
  flex: 1;
  min-width: 0;
  overflow: hidden;
  padding-left: 8px;
  padding-right: 8px;
}

.group-name {
  font-weight: 500;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.group-now {
  font-size: 12px;
  color: var(--accent);
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.group-actions {
  margin-left: auto;
  flex: none;
  display: flex;
  align-items: center;
  gap: 2px;
}

.group-actions .el-button + .el-button {
  margin-left: 0;
}

.proxy-list {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin: 0;
  padding: 4px 0 0;
}

/* button 元素天然支持 Tab/Enter/Space；重置 UA 默认样式以保持原视觉。 */
.proxy-item {
  appearance: none;
  font-family: inherit;
  text-align: left;
  list-style: none;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border-radius: var(--r-sm);
  background: var(--bg-raised);
  border: 1px solid var(--card-border);
  color: var(--text-secondary);
  font-size: 13px;
  line-height: inherit;
  cursor: pointer;
  user-select: none;
  transition:
    background-color 0.18s ease,
    border-color 0.18s ease,
    color 0.18s ease,
    transform 0.12s ease;
}

.proxy-node-delay {
  font-size: 12px;
  color: var(--text-tertiary);
}

.proxy-item:hover {
  background: var(--interactive-hover);
  border-color: var(--border-subtle);
  color: var(--text-primary);
}

.proxy-item:active {
  transform: translateY(1px);
}

.proxy-item.active {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-soft);
  font-weight: 500;
}

.proxy-item.active .proxy-node-delay {
  color: var(--accent);
}

.proxy-item:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
</style>
