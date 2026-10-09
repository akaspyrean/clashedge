<!-- src/components/ui/CeLatencyTag.vue - 延迟标签：形状 + 颜色 + 文字三重编码（§3.4）。
     clickable 时渲染为 <button>，单击发出 retest。 -->
<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { classifyLatency } from "@/utils/latency";

const props = defineProps<{
  ms?: number | null;
  testing?: boolean;
  timeout?: boolean;
  /** 自定义阈值（来自设置） */
  good?: number;
  bad?: number;
  /** 节点名，仅用于「重新测速：{name}」的读屏文案 */
  name?: string;
  clickable?: boolean;
}>();
const emit = defineEmits<{ retest: [] }>();
const { t } = useI18n();

const kind = computed(() =>
  classifyLatency({
    ms: props.ms,
    testing: props.testing,
    timeout: props.timeout,
    thresholds: { good: props.good, bad: props.bad },
  }),
);

const text = computed(() => {
  switch (kind.value) {
    case "good":
    case "mid":
    case "bad":
      return `${Math.round(props.ms as number)} ms`;
    case "timeout":
      return t("ui.latency.timeout");
    case "testing":
      return t("ui.latency.testing");
    default:
      return "– ms";
  }
});

const description = computed(() => {
  switch (kind.value) {
    case "good":
    case "mid":
    case "bad":
      return t("ui.latency.aria", {
        ms: Math.round(props.ms as number),
        level: t(`ui.latency.${kind.value}`),
      });
    case "timeout":
      return t("ui.latency.aria_timeout");
    case "testing":
      return t("ui.latency.aria_testing");
    default:
      return t("ui.latency.aria_none");
  }
});

const label = computed(() =>
  props.clickable
    ? `${description.value}，${t("ui.latency.retest", { name: props.name ?? "" })}`.trim()
    : description.value,
);
</script>

<template>
  <button
    v-if="clickable"
    type="button"
    class="ce-lat"
    :class="`ce-lat--${kind}`"
    :aria-label="label"
    :aria-busy="kind === 'testing' || undefined"
    @click.stop="emit('retest')"
  >
    {{ text }}
  </button>
  <span v-else class="ce-lat" :class="`ce-lat--${kind}`" role="img" :aria-label="label">{{ text }}</span>
</template>

<style scoped>
.ce-lat {
  display: inline-flex;
  align-items: center;
  gap: var(--ce-space-2);
  height: 22px;
  padding: 0 var(--ce-space-2);
  border: 0;
  border-radius: var(--ce-radius-sm);
  font: var(--ce-type-caption);
  font-family: var(--ce-font-mono);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

button.ce-lat {
  cursor: pointer;
}

button.ce-lat:focus-visible {
  outline: 2px solid var(--ce-accent-bg);
  outline-offset: 2px;
}

.ce-lat::before {
  content: "";
  flex: none;
  width: 8px;
  height: 8px;
}

/* 良好：圆 */
.ce-lat--good {
  background: var(--ce-success-soft);
  color: var(--ce-success-fg);
}
.ce-lat--good::before {
  border-radius: 50%;
  background: var(--ce-success-solid);
}

/* 一般：菱形 */
.ce-lat--mid {
  background: var(--ce-warning-soft);
  color: var(--ce-warning-fg);
}
.ce-lat--mid::before {
  width: 7px;
  height: 7px;
  transform: rotate(45deg);
  background: var(--ce-warning-solid);
}

/* 较差：横杠 */
.ce-lat--bad {
  background: var(--ce-danger-soft);
  color: var(--ce-danger-fg);
}
.ce-lat--bad::before {
  width: 9px;
  height: 3px;
  border-radius: var(--ce-radius-full);
  background: var(--ce-danger-solid);
}

/* 超时：无形状 */
.ce-lat--timeout {
  background: var(--ce-fill-soft);
  color: var(--ce-text-secondary);
  font-family: var(--ce-font-sans);
}
.ce-lat--timeout::before,
.ce-lat--none::before {
  display: none;
}

/* 测速中：转圈 */
.ce-lat--testing {
  background: var(--ce-pending-soft);
  color: var(--ce-pending-fg);
  font-family: var(--ce-font-sans);
}
.ce-lat--testing::before {
  border-radius: 50%;
  border: 2px solid var(--ce-pending-solid);
  border-right-color: transparent;
  animation: ce-lat-spin 0.9s linear infinite;
}

/* 未测：虚线描边 */
.ce-lat--none {
  background: transparent;
  color: var(--ce-text-tertiary);
  border: 1px dashed var(--ce-text-disabled);
  padding: 0 calc(var(--ce-space-2) - 1px);
}

@keyframes ce-lat-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
