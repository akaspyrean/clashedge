<!-- src/components/ui/CeStatusCore.vue - 主连接按钮 + 状态文字（直径 72）。
     状态：stopped / starting / running / error；starting 时禁用并 aria-busy。
     默认插槽放状态文字下方的补充信息（版本、端口、操作按钮）。 -->
<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Power, TriangleAlert } from "lucide-vue-next";

export type CoreState = "stopped" | "starting" | "running" | "error";

const props = defineProps<{ state: CoreState; /** 覆盖默认状态文字（如「核心运行中，尚未接管流量」） */ label?: string }>();
const emit = defineEmits<{ toggle: [] }>();
const { t } = useI18n();

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
    <div class="ce-core__text">
      <span class="ce-core__status" role="status">{{ statusText }}</span>
      <slot />
    </div>
  </div>
</template>

<style scoped>
.ce-core {
  display: flex;
  align-items: center;
  gap: var(--ce-space-5);
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
  box-shadow: 0 0 0 5px color-mix(in srgb, var(--ce-pending-solid) 30%, transparent);
  cursor: progress;
}

.ce-core__button.is-error {
  border: 2px solid var(--ce-danger-solid);
  background: var(--ce-danger-soft);
  color: var(--ce-danger-fg);
}

.ce-core__text {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-1);
  min-width: 0;
}

.ce-core__status {
  font: var(--ce-type-title-2);
  color: var(--ce-text-primary);
}

[data-state="error"] .ce-core__status {
  color: var(--ce-danger-fg);
}

[data-state="starting"] .ce-core__status {
  color: var(--ce-pending-fg);
}
</style>
