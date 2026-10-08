<!-- src/components/StatusBanner.vue - 页面顶部横幅（核心错误 / 配置降级等）统一外观。
     el-alert 自带 role="alert"，读屏器会立即朗读。 -->
<script setup lang="ts">
defineProps<{
  title: string;
  type?: "info" | "success" | "warning" | "error";
}>();
</script>

<template>
  <el-alert :title="title" :type="type ?? 'warning'" show-icon :closable="false" class="status-banner">
    <slot />
  </el-alert>
</template>

<style scoped>
.status-banner {
  margin-bottom: 12px;
  overflow-wrap: anywhere;
}
</style>
