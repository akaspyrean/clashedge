<!-- src/components/FormDialog.vue - 表单对话框：统一宽度、页脚（取消 / 提交）、提交中状态。
     el-dialog 自带焦点陷阱与关闭后焦点回到触发元素，标题通过 aria-labelledby 命名对话框。 -->
<script setup lang="ts">
import { useI18n } from "vue-i18n";

withDefaults(
  defineProps<{
    modelValue: boolean;
    title: string;
    /** 提交按钮文案；缺省"保存" */
    submitLabel?: string;
    submitting?: boolean;
    submitDisabled?: boolean;
    /** 不需要提交（只读展示）时隐藏提交按钮 */
    hideSubmit?: boolean;
  }>(),
  { submitLabel: undefined, submitting: false, submitDisabled: false, hideSubmit: false },
);
const emit = defineEmits<{
  (e: "update:modelValue", v: boolean): void;
  (e: "submit"): void;
  (e: "open"): void;
}>();
const { t } = useI18n();
</script>

<template>
  <el-dialog
    :model-value="modelValue"
    :title="title"
    width="min(640px, calc(100vw - 32px))"
    @update:model-value="emit('update:modelValue', $event)"
    @open="emit('open')"
  >
    <slot />
    <template #footer>
      <el-button @click="emit('update:modelValue', false)">{{ t("profiles.cancel") }}</el-button>
      <el-button
        v-if="!hideSubmit"
        type="primary"
        :loading="submitting"
        :disabled="submitDisabled"
        @click="emit('submit')"
      >
        {{ submitLabel ?? t("profiles.save") }}
      </el-button>
    </template>
  </el-dialog>
</template>
