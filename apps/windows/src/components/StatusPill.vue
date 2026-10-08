<!-- src/components/StatusPill.vue - 状态胶囊（运行中 / 已连接 …）。
     视觉由全局 .status-pill / .status-dot（styles.css）提供；这里补语义：
     role="status"（礼貌的 live region），状态变化会被读屏器朗读，颜色之外有文字。 -->
<script setup lang="ts">
defineProps<{ active: boolean; label: string }>();
</script>

<template>
  <span class="status-pill" :class="{ running: active }" role="status">
    <span class="status-dot" aria-hidden="true"></span>
    {{ label }}
  </span>
</template>
