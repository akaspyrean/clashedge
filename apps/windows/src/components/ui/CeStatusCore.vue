<!-- src/components/ui/CeStatusCore.vue - 主连接按钮 + 状态文字（直径 72）。
     状态：stopped / starting / running / error；starting 时禁用并 aria-busy。
     插槽：
       - default: 兼容旧用法，放在状态文字下方（右对齐）
       - info: 中部两行文本（左对齐），如版本/端口
       - actions: 右侧操作按钮区 -->
<script setup lang="ts">
import { computed, useSlots } from "vue";
import { useI18n } from "vue-i18n";
import { Power, TriangleAlert, AlertCircle } from "lucide-vue-next";

export type CoreState = "stopped" | "starting" | "running" | "error";

const props = defineProps<{
  state: CoreState;
  /** 覆盖默认状态文字（如「核心运行中，尚未接管流量」） */
  label?: string;
  /** 错误详情（error 状态下显示） */
  errorMessage?: string;
}>();
const emit = defineEmits<{ toggle: [] }>();
const { t } = useI18n();
const slots = useSlots();

const statusText = computed(() => props.label ?? t(`ui.core.${props.state}`));
const ariaLabel = computed(() => {
  switch (props.state) {
    case "running":
      return t("ui.core.aria_stop");
    case "starting":
      return t("ui.core.aria_starting");
    case "error":
      return t("ui.core.aria_error");
    default:
      return t("ui.core.aria_start");
  }
});

const hasInfoSlot = computed(() => !!slots.info);
const hasActionsSlot = computed(() => !!slots.actions);
const showErrorDetail = computed(() => props.state === "error" && props.errorMessage);
</script>

<template>
  <div class="ce-core" :data-state="state">
    <button
      type="button"
      class="ce-core__button"
      :class="`is-${state}`"
      :aria-label="ariaLabel"
      :aria-busy="state === 'starting' ? 'true' : undefined"
      :disabled="state === 'starting'"
      @click="emit('toggle')"
    >
      <TriangleAlert v-if="state === 'error'" :size="24" :stroke-width="1.75" aria-hidden="true" />
      <Power v-else :size="24" :stroke-width="1.75" aria-hidden="true" />
    </button>

    <div class="ce-core__middle" v-if="hasInfoSlot">
      <slot name="info" />
    </div>

    <div class="ce-core__text">
      <span v-if="!hasActionsSlot" class="ce-core__status" role="status">{{ statusText }}</span>
      <slot v-if="!hasInfoSlot && !hasActionsSlot" />
      <slot name="actions" :statusText="statusText" v-if="hasActionsSlot" />
    </div>

    <!-- 错误详情：error 状态且有错误信息时显示 -->
    <div v-if="showErrorDetail" class="ce-core__error-detail" role="alert">
      <AlertCircle :size="16" :stroke-width="1.75" aria-hidden="true" />
      <span>{{ errorMessage }}</span>
    </div>
  </div>
</template>

<style scoped>
.ce-core {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ce-space-5);
  width: 100%;
}

.ce-core__button {
  display: grid;
  place-items: center;
  flex: none;
  width: 72px;
  height: 72px;
  padding: 0;
  border: 1px solid var(--ce-border);
  border-radius: 50%;
  background: var(--ce-bg-surface);
  color: var(--ce-text-secondary);
  cursor: pointer;
  transition:
    transform var(--ce-dur-fast) var(--ce-ease-spring),
    box-shadow var(--ce-dur-base) var(--ce-ease-standard),
    background-color var(--ce-dur-base) var(--ce-ease-standard),
    color var(--ce-dur-base) var(--ce-ease-standard),
    border-color var(--ce-dur-base) var(--ce-ease-standard);
}

/* ---- hover：四态各自反馈 ---- */

/* stopped：底色轻微加深 + 文字提亮 */
.ce-core__button:hover:not(:disabled):not(.is-running):not(.is-error) {
  background: var(--ce-fill-hover);
  color: var(--ce-text-primary);
  border-color: var(--ce-text-tertiary);
}

/* running：品牌色加深 + 外环收紧 */
.ce-core__button.is-running:hover {
  background: var(--ce-accent-bg-hover);
  box-shadow: 0 0 0 6px var(--ce-accent-soft);
}

/* error：边框加粗 + 背景加重 */
.ce-core__button.is-error:hover {
  border-width: 2px;
  background: color-mix(in srgb, var(--ce-danger-soft) 70%, var(--ce-bg-surface));
}

/* starting 禁用，无 hover */

.ce-core__button:active:not(:disabled) {
  transform: scale(0.97);
}

.ce-core__button:focus-visible {
  outline: 2px solid var(--ce-accent-bg);
  outline-offset: 4px;
}

.ce-core__button.is-running {
  border-color: transparent;
  background: var(--ce-accent-bg);
  color: var(--ce-on-accent);
  box-shadow: 0 0 0 6px var(--ce-accent-soft);
}

.ce-core__button.is-starting {
  border-color: transparent;
  background: var(--ce-pending-soft);
  color: var(--ce-pending-fg);
  box-shadow: 0 0 0 6px color-mix(in srgb, var(--ce-pending-solid) 30%, transparent);
  cursor: progress;
}

.ce-core__button.is-error {
  border: 2px solid var(--ce-danger-solid);
  background: var(--ce-danger-soft);
  color: var(--ce-danger-fg);
}

/* 中部信息区：两行文本，左对齐 */
.ce-core__middle {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  text-align: left;
}

.ce-core__middle > * {
  line-height: 1.4;
}

/* 右侧文字 + 操作区 */
.ce-core__text {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: var(--ce-space-2);
  min-width: 0;
  text-align: right;
}

.ce-core__status {
  font: var(--ce-type-title-3);
  color: var(--ce-text-primary);
}

[data-state="error"] .ce-core__status {
  color: var(--ce-danger-fg);
}

[data-state="starting"] .ce-core__status {
  color: var(--ce-pending-fg);
}

/* 操作按钮区（actions 插槽） */
.ce-core__text :slotted(.ce-core__actions) {
  display: flex;
  gap: var(--ce-space-2);
  margin-top: var(--ce-space-2);
}

/* 错误详情：紧凑的红色提示行 */
.ce-core__error-detail {
  display: flex;
  align-items: center;
  gap: var(--ce-space-2);
  padding: var(--ce-space-2) var(--ce-space-3);
  background: var(--ce-danger-soft);
  border: 1px solid var(--ce-danger-border, var(--ce-danger-solid));
  border-radius: var(--ce-radius-md);
  color: var(--ce-danger-fg);
  font: var(--ce-type-callout);
  line-height: 1.4;
  animation: ce-core__error-slide var(--ce-dur-base) var(--ce-ease-standard);
}

@keyframes ce-core__error-slide {
  from {
    opacity: 0;
    transform: translateY(-4px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

/* 响应式：窄屏堆叠 */
@media (max-width: 599px) {
  .ce-core {
    flex-wrap: wrap;
    gap: var(--ce-space-4);
  }
  .ce-core__button {
    margin: 0 auto;
  }
  .ce-core__middle {
    order: 1;
    width: 100%;
    text-align: center;
  }
  .ce-core__text {
    order: 2;
    width: 100%;
    align-items: center;
    text-align: center;
  }
}
</style>
