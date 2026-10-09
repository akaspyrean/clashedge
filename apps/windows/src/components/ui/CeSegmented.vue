<!-- src/components/ui/CeSegmented.vue - 分段控件（role=radiogroup）。
     方向键切换并选中，Home/End 跳到首尾；选中项 tabindex=0（roving tabindex）。 -->
<script setup lang="ts" generic="T extends string | number">
import { nextTick, ref } from "vue";

export interface SegmentedOption<V> {
  value: V;
  label: string;
  disabled?: boolean;
}

const props = defineProps<{
  modelValue: T;
  options: SegmentedOption<T>[];
  ariaLabel: string;
}>();
const emit = defineEmits<{ "update:modelValue": [value: T] }>();

const buttons = ref<HTMLButtonElement[]>([]);

function enabledIndexes(): number[] {
  return props.options.flatMap((o, i) => (o.disabled ? [] : [i]));
}

function select(i: number) {
  const opt = props.options[i];
  if (!opt || opt.disabled) return;
  if (opt.value !== props.modelValue) emit("update:modelValue", opt.value);
}

async function move(from: number, step: 1 | -1 | "first" | "last") {
  const idx = enabledIndexes();
  if (idx.length === 0) return;
  let target: number;
  if (step === "first") target = idx[0];
  else if (step === "last") target = idx[idx.length - 1];
  else {
    const pos = idx.indexOf(from);
    target = idx[(pos + step + idx.length) % idx.length];
  }
  select(target);
  await nextTick();
  buttons.value[target]?.focus();
}

function onKeydown(e: KeyboardEvent, i: number) {
  const map: Record<string, 1 | -1 | "first" | "last"> = {
    ArrowRight: 1,
    ArrowDown: 1,
    ArrowLeft: -1,
    ArrowUp: -1,
    Home: "first",
    End: "last",
  };
  const step = map[e.key];
  if (step === undefined) return;
  e.preventDefault();
  void move(i, step);
}
</script>

<template>
  <div class="ce-segmented" role="radiogroup" :aria-label="ariaLabel">
    <button
      v-for="(opt, i) in options"
      :key="String(opt.value)"
      :ref="(el) => (buttons[i] = el as HTMLButtonElement)"
      type="button"
      role="radio"
      class="ce-segmented__item"
      :aria-checked="opt.value === modelValue"
      :tabindex="opt.value === modelValue ? 0 : -1"
      :disabled="opt.disabled"
      @click="select(i)"
      @keydown="onKeydown($event, i)"
    >
      {{ opt.label }}
    </button>
  </div>
</template>

<style scoped>
.ce-segmented {
  display: inline-flex;
  gap: 2px;
  padding: 3px;
  background: var(--ce-fill-soft);
  border-radius: var(--ce-radius-full);
}

.ce-segmented__item {
  height: 30px;
  padding: 0 var(--ce-space-4);
  border: 0;
  border-radius: var(--ce-radius-full);
  background: transparent;
  color: var(--ce-text-secondary);
  font: var(--ce-type-callout);
  font-weight: 500;
  white-space: nowrap;
  cursor: pointer;
  transition:
    background-color var(--ce-dur-fast) var(--ce-ease-standard),
    color var(--ce-dur-fast) var(--ce-ease-standard);
}

.ce-segmented__item:hover:not([aria-checked="true"]):not(:disabled) {
  color: var(--ce-text-primary);
}

.ce-segmented__item[aria-checked="true"] {
  background: var(--ce-bg-surface);
  color: var(--ce-text-primary);
  box-shadow: var(--ce-shadow-e1);
}

.ce-segmented__item:disabled {
  color: var(--ce-text-disabled);
  cursor: not-allowed;
}

.ce-segmented__item:focus-visible {
  outline: 2px solid var(--ce-accent-bg);
  outline-offset: 2px;
}
</style>
