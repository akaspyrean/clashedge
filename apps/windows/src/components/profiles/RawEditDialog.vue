<!-- 原始内容编辑：打开时读取，保存时写回（激活中的配置由后端热重载生效，失败回滚）。 -->
<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import FormDialog from "@/components/FormDialog.vue";
import { useAction } from "@/composables/useAction";
import { useNotify } from "@/composables/useNotify";
import { useProfilesStore } from "@/stores/profiles";

const props = defineProps<{ target: string | null }>();
const visible = defineModel<boolean>({ required: true });
const { t } = useI18n();
const profiles = useProfilesStore();
const notify = useNotify();
const { busy, run } = useAction();

const content = ref("");
/** 读取失败时禁止保存：否则会用空内容覆盖原文件。 */
const loaded = ref(false);

async function onOpen() {
  content.value = "";
  loaded.value = false;
  if (props.target === null) return;
  try {
    content.value = await profiles.getContent(props.target);
    loaded.value = true;
  } catch (e) {
    notify.fail(e);
  }
}

async function submit() {
  if (props.target === null || !loaded.value) return;
  const target = props.target;
  const r = await run(() => profiles.updateContent(target, content.value), { success: true });
  if (r.ok) visible.value = false;
}
</script>

<template>
  <FormDialog
    v-model="visible"
    :title="t('profiles.raw_edit')"
    :submitting="busy"
    :submit-disabled="!loaded"
    @open="onOpen"
    @submit="submit"
  >
    <el-input v-model="content" type="textarea" :rows="16" class="mono" :aria-label="t('profiles.content')" />
  </FormDialog>
</template>

<style scoped>
.mono :deep(textarea) {
  font-family: "Consolas", "Menlo", monospace;
}
</style>
