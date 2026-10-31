<!-- src/components/ui/CeNodeTile.vue - 节点格。
     结构：外层容器 + 覆盖整格的主按钮（选中节点）+ 独立的延迟标签按钮（重新测速），
     避免按钮嵌套按钮。readonly（自动测速组）只展示不可点选。 -->
<script setup lang="ts">
import { computed } from "vue";
import { Check } from "lucide-vue-next";
import CeLatencyTag from "./CeLatencyTag.vue";
import { formatNodeDisplay } from "@/utils/format";

const props = defineProps<{
  name: string;
  protocol?: string;
  ms?: number | null;
  testing?: boolean;
  timeout?: boolean;
  good?: number;
  bad?: number;
  selected?: boolean;
  /** 节点地区（由后端按节点名推断；识别不到则为 undefined，不显示） */
  region?: string;
  /** 节点不可用（延迟失败等）：文字弱化为 disabled 色，仍可点选 */
  unavailable?: boolean;
  /** 只读：自动测速组由内核自动选择 */
  readonly?: boolean;
}>();
const emit = defineEmits<{ select: []; retest: [] }>();

const formatted = computed(() => formatNodeDisplay(props.name, props.region));

function onSelect() {
  if (!props.readonly) emit("select");
}
</script>

<template>
  <div
    class="ce-tile"
    :class="{ 'is-selected': selected, 'is-unavailable': unavailable, 'is-readonly': readonly }"
  >
    <button
      type="button"
      class="ce-tile__main"
      :aria-pressed="selected ? 'true' : 'false'"
      :aria-disabled="readonly ? 'true' : undefined"
      :title="name"
      @click="onSelect"
    >
      <div class="ce-tile__header">
        <span class="ce-tile__name">{{ formatted.displayName }}</span>
        <span v-if="formatted.regionCode" class="ce-tile__badge" aria-hidden="true">{{ formatted.regionCode }}</span>
      </div>
      <span v-if="name !== formatted.displayName" class="ce-tile__subname" :title="name">{{ name }}</span>
      <span v-if="protocol" class="ce-tile__proto">{{ protocol }}</span>
    </button>
    <!-- 地区与延迟同行：地区在延迟标签左侧 -->
    <div class="ce-tile__meta">
      <span v-if="region" class="ce-tile__region" :title="region">{{ region }}</span>
      <CeLatencyTag
        class="ce-tile__lat"
        clickable
        :name="name"
        :ms="ms"
        :testing="testing"
        :timeout="timeout"
        :good="good"
        :bad="bad"
        @retest="emit('retest')"
      />
    </div>
    <span v-if="selected" class="ce-tile__check" aria-hidden="true"><Check :size="12" :stroke-width="2.5" /></span>
  </div>
</template>

<style scoped>
.ce-tile {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-2);
  min-width: 200px;
  min-height: 72px;
  padding: var(--ce-space-3) var(--ce-space-4);
  border: 1px solid var(--ce-border);
  border-radius: calc(var(--ce-radius-lg) - 2px);
  background: var(--ce-bg-surface);
  color: var(--ce-text-primary);
  transition:
    border-color var(--ce-dur-fast) var(--ce-ease-standard),
    box-shadow var(--ce-dur-fast) var(--ce-ease-standard);
}

.ce-tile:hover {
  border-color: var(--ce-text-disabled);
}

.ce-tile.is-selected,
.ce-tile.is-selected:hover {
  border-color: var(--ce-accent-bg);
  background: var(--ce-accent-soft);
  box-shadow: 0 0 0 0.5px var(--ce-accent-bg);
}

.ce-tile__main {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-2);
  width: 100%;
  min-width: 0;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  font: var(--ce-type-body);
  text-align: left;
  cursor: pointer;
}

/* 覆盖整格的点击区域；延迟标签用 z-index 抬到其上。 */
.ce-tile__main::after {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: inherit;
}

.ce-tile__main:focus-visible {
  outline: none;
}

.ce-tile__main:focus-visible::after {
  outline: 2px solid var(--ce-accent-bg);
  outline-offset: 2px;
}

.ce-tile__header {
  display: flex;
  align-items: center;
  gap: var(--ce-space-2);
  width: 100%;
  min-width: 0;
}

.ce-tile__name {
  font-weight: 500;
  line-height: 20px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ce-tile__subname {
  font-family: var(--ce-font-mono);
  font-size: 11px;
  line-height: 14px;
  color: var(--ce-text-disabled);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ce-tile__badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 1px 5px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  font-size: 10px;
  font-weight: 700;
  line-height: 1;
  border-radius: 4px;
  background: var(--ce-accent-soft);
  color: var(--ce-accent-fg);
  border: 1px solid var(--ce-border);
  flex-shrink: 0;
}

.ce-tile.is-selected .ce-tile__name {
  color: var(--ce-accent-fg);
  /* 让位给右上角 16px 勾选徽标（right 12 + 16 + 间隙 4） */
  padding-right: 32px;
}

.ce-tile__proto {
  font: var(--ce-type-caption);
  font-weight: 400;
  color: var(--ce-text-tertiary);
}

/* 地区 + 延迟固定在卡片底部同一行：地区占左侧剩余宽度、延迟靠右对齐。
   两者都必须参与 flex 流；任何一方再绝对定位都会互相覆盖并压住节点名。 */
.ce-tile__meta {
  position: relative; /* 保持在 .ce-tile__main::after 点击遮罩之上 */
  z-index: 1;
  display: flex;
  align-items: center;
  gap: var(--ce-space-2);
  margin-top: auto;
  width: 100%;
  min-width: 0;
}

.ce-tile__region {
  flex: 1 1 auto;
  min-width: 0;
  font: var(--ce-type-caption);
  color: var(--ce-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 延迟标签作为 meta 行的右侧项参与布局（无地区时也能贴右）。 */
.ce-tile__lat {
  flex: none;
  margin-left: auto;
}

.ce-tile.is-unavailable .ce-tile__region {
  color: var(--ce-text-disabled);
}

.ce-tile__check {
  position: absolute;
  top: var(--ce-space-3);
  right: var(--ce-space-3);
  display: grid;
  place-items: center;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--ce-accent-bg);
  color: var(--ce-on-accent);
  pointer-events: none;
}

.ce-tile.is-unavailable .ce-tile__name,
.ce-tile.is-unavailable .ce-tile__proto {
  color: var(--ce-text-disabled);
}

.ce-tile.is-readonly .ce-tile__main {
  cursor: default;
}
</style>
