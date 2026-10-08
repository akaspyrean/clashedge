<!-- 新建 / 导入配置：名称 + 粘贴 YAML 内容（两者表单相同，仅标题与提交动作不同）。 -->
<script setup lang="ts">
import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import FormDialog from "@/components/FormDialog.vue";
import { useAction } from "@/composables/useAction";
import { useProfilesStore } from "@/stores/profiles";

const props = defineProps<{ mode: "create" | "import" }>();
const visible = defineModel<boolean>({ required: true });
const { t } = useI18n();
const profiles = useProfilesStore();
const { busy, run } = useAction();

const name = ref("");
const content = ref("");

watch(visible, (v) => {
  if (v) name.value = content.value = "";
});

async function submit() {
  const n = name.value.trim();
  if (!n) return;
  const r = await run(
    () => (props.mode === "create" ? profiles.create(n, content.value) : profiles.importContent(n, content.value)),
    { success: true },
  );
  if (r.ok) visible.value = false;
}
</script>

<template>
  <FormDialog
    v-model="visible"
    :title="mode === 'create' ? t('profiles.new') : t('profiles.import')"
    :submitting="busy"
    :submit-disabled="!name.trim()"
    @submit="submit"
  >
    <el-form label-position="top" @submit.prevent>
      <el-form-item :label="t('profiles.name')">
        <el-input v-model="name" :placeholder="t('profiles.name')" />
      </el-form-item>
      <el-form-item :label="t('profiles.content')">
        <el-input v-model="content" type="textarea" :rows="10" class="mono" />
      </el-form-item>
    </el-form>
  </FormDialog>
</template>

<style scoped>
.mono :deep(textarea) {
  font-family: "Consolas", "Menlo", monospace;
}
</style>
