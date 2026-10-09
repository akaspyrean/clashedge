<!-- src/components/ui/CeNodeTile.vue - 节点格。
     结构：外层容器 + 覆盖整格的主按钮（选中节点）+ 独立的延迟标签按钮（重新测速），
     避免按钮嵌套按钮。readonly（自动测速组）只展示不可点选。 -->
<script setup lang="ts">
import { Check } from "lucide-vue-next";
import CeLatencyTag from "./CeLatencyTag.vue";

const props = defineProps<{
  name: string;
  protocol?: string;
  ms?: number | null;
  testing?: boolean;
  timeout?: boolean;
  good?: number;
  bad?: number;
  selected?: boolean;
  /** 节点不可用（延迟失败等）：文字弱化为 disabled 色，仍可点选 */
  unavailable?: boolean;
  /** 只读：自动测速组由内核自动选择 */
  readonly?: boolean;
}>();
const emit = defineEmits<{ select: []; retest: [] }>();

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
      @click="onSelect"
    >
      <span class="ce-tile__name">{{ name }}</span>
      <span v-if="protocol" class="ce-tile__proto">{{ protocol }}</span>
    </button>
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
    <span v-if="selected" class="ce-tile__check" aria-hidden="true"><Check :size="12" :stroke-width="2.5" /></span>
  </div>
</template>

<style scoped>
.ce-tile {
  position: relative;
  min-width: 200px;
  min-height: 72px;
  padding: var(--ce-space-3) 14px;
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
  box-shadow: var(--ce-shadow-e1);
}

.ce-tile.is-selected,
.ce-tile.is-selected:hover {
  border-color: var(--ce-accent-bg);
  box-shadow: 0 0 0 0.5px var(--ce-accent-bg);
}

.ce-tile__main {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-2);
  width: 100%;
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

.ce-tile__name {
  font-weight: 500;
  line-height: 20px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ce-tile.is-selected .ce-tile__name {
  color: var(--ce-accent-fg);
  padding-right: var(--ce-space-5);
}

.ce-tile__proto {
  font: var(--ce-type-caption);
  font-weight: 400;
  color: var(--ce-text-tertiary);
}

.ce-tile__lat {
  position: absolute;
  right: 14px;
  bottom: var(--ce-space-3);
  z-index: 1;
}

.ce-tile__check {
  position: absolute;
  top: 10px;
  right: 10px;
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
