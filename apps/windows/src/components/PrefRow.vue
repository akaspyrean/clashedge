<!-- src/components/PrefRow.vue - 偏好行：标题(+副文本) 居左、控件 居右。
     无障碍：行本身是 role="group"，由标题 aria-labelledby 命名；控件通过作用域插槽拿到
     `label`（标题文本）/ `hintId`，用于 `:aria-label` / `aria-describedby`，
     这样读屏器读到的是「语言 下拉框」而不是一个无名控件。 -->
<script setup lang="ts">
import { useId } from "vue";

defineProps<{
  /** 标题（已本地化文本） */
  title: string;
  /** 副文本（可选） */
  hint?: string;
  /** 宽控件（如 URL 输入框）：吃满行内剩余宽度 */
  wide?: boolean;
}>();

const uid = useId();
const labelId = `pref-${uid}-label`;
const hintId = `pref-${uid}-hint`;
</script>

<template>
  <div class="pref-row" role="group" :aria-labelledby="labelId">
    <div class="pref-info">
      <div :id="labelId" class="pref-title">{{ title }}</div>
      <div v-if="hint" :id="hintId" class="pref-hint">{{ hint }}</div>
      <slot name="info" />
    </div>
    <div class="pref-control" :class="{ 'pref-control-wide': wide }">
      <slot :label="title" :hint-id="hint ? hintId : undefined" />
    </div>
  </div>
</template>

<style scoped>
.pref-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 12px 0;
  min-height: 44px;
}

.pref-info {
  min-width: 0;
}

.pref-title {
  font-size: 14px;
  font-weight: 500;
  color: var(--text-primary);
}

.pref-hint {
  margin-top: 2px;
  font-size: 12px;
  color: var(--text-tertiary);
}

.pref-control {
  flex: none;
  display: flex;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 8px;
}

.pref-control-wide {
  flex: 1;
  min-width: 0;
}

.pref-control-wide :deep(.el-input) {
  width: 100%;
}
</style>
