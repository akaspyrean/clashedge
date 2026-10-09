<!-- src/components/PrefList.vue - 偏好行列表容器：行间细分隔线；可选"保存"行与底部区。 -->
<script setup lang="ts">
defineProps<{
  /** 显示右对齐的主按钮"保存"行 */
  saveLabel?: string;
}>();
defineEmits<{ (e: "save"): void }>();
</script>

<template>
  <div class="pref-list">
    <slot />
    <div v-if="saveLabel" class="pref-save-row">
      <el-button type="primary" @click="$emit('save')">{{ saveLabel }}</el-button>
    </div>
    <div v-if="$slots.footer" class="pref-footer-row">
      <slot name="footer" />
    </div>
  </div>
</template>

<style scoped>
.pref-list {
  width: 100%;
}

/* 相邻行（含插槽里渲染的 PrefRow 根节点）之间的分隔线 */
.pref-list :deep(.pref-row + .pref-row) {
  border-top: 1px solid var(--ce-border);
}

.pref-save-row {
  display: flex;
  justify-content: flex-end;
  padding-top: 16px;
}

/* 底部独立区域（导入 / 导出 / 重置等），顶部细分隔线。 */
.pref-footer-row {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 8px;
  padding-top: 16px;
  border-top: 1px solid var(--ce-border);
}
</style>
