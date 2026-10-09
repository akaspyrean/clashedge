<!-- src/components/StatusPill.vue - 状态胶囊（运行中 / 已连接 …）。
     三重编码：色点形状（圆 / 菱形 / 横杠 / 转圈 / 空心圆）+ 颜色 + 文字，不只靠颜色。
     role="status"（礼貌的 live region），状态变化会被读屏器朗读。
     tone 缺省时由 active 推断：true → ok，false → idle。 -->
<script setup lang="ts">
import { computed } from "vue";

export type StatusTone = "ok" | "warn" | "error" | "pending" | "idle";

const props = defineProps<{ active: boolean; label: string; tone?: StatusTone }>();
const resolved = computed<StatusTone>(() => props.tone ?? (props.active ? "ok" : "idle"));
</script>

<template>
  <span class="status-pill" :class="[`tone-${resolved}`, { running: resolved === 'ok' }]" role="status">
    <span class="status-dot" aria-hidden="true"></span>
    {{ label }}
  </span>
</template>

<style scoped>
.status-pill {
  display: inline-flex;
  align-items: center;
  gap: var(--ce-space-2);
  height: 22px;
  padding: 0 10px 0 var(--ce-space-2);
  border-radius: var(--ce-radius-full);
  font: var(--ce-type-caption);
  white-space: nowrap;
}

.status-dot {
  flex: none;
  width: 8px;
  height: 8px;
}

.tone-ok {
  background: var(--ce-success-soft);
  color: var(--ce-success-fg);
}
.tone-ok .status-dot {
  border-radius: 50%;
  background: var(--ce-success-solid);
}

.tone-warn {
  background: var(--ce-warning-soft);
  color: var(--ce-warning-fg);
}
.tone-warn .status-dot {
  width: 7px;
  height: 7px;
  transform: rotate(45deg);
  background: var(--ce-warning-solid);
}

.tone-error {
  background: var(--ce-danger-soft);
  color: var(--ce-danger-fg);
}
.tone-error .status-dot {
  width: 9px;
  height: 3px;
  border-radius: var(--ce-radius-full);
  background: var(--ce-danger-solid);
}

.tone-pending {
  background: var(--ce-pending-soft);
  color: var(--ce-pending-fg);
}
.tone-pending .status-dot {
  border-radius: 50%;
  border: 2px solid var(--ce-pending-solid);
  border-right-color: transparent;
  animation: status-pill-spin 0.9s linear infinite;
}

.tone-idle {
  background: var(--ce-fill-soft);
  color: var(--ce-idle);
}
.tone-idle .status-dot {
  border-radius: 50%;
  border: 1.5px solid var(--ce-idle);
}

@keyframes status-pill-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
