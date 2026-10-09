<!-- 导出配置：选择配置文件 → 显示其 YAML 文本（只读，供复制）。 -->
<script setup lang="ts">
import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import FormDialog from "@/components/FormDialog.vue";
import { useAction } from "@/composables/useAction";
import { useProfilesStore } from "@/stores/profiles";

const visible = defineModel<boolean>({ required: true });
const props = defineProps<{ /** 从某张卡片的「导出」进入时预选该配置 */ initial?: string | null }>();
const { t } = useI18n();
const profiles = useProfilesStore();
const { busy, run } = useAction();

const target = ref("");
const content = ref("");

watch(visible, (v) => {
  if (v) {
    content.value = "";
    target.value = props.initial ?? profiles.profiles[0]?.name ?? "";
  }
});

async function submit() {
  if (!target.value) return;
  const r = await run(() => profiles.exportContent(target.value));
  if (r.ok) content.value = r.value;
}
</script>

<template>
  <FormDialog
    v-model="visible"
    :title="t('profiles.export')"
    :submit-label="t('profiles.export')"
    :submitting="busy"
    :submit-disabled="!target"
    @submit="submit"
  >
    <el-form label-position="top" @submit.prevent>
      <el-form-item :label="t('profiles.name')">
        <el-select v-model="target" style="width: 100%">
          <el-option v-for="p in profiles.profiles" :key="p.name" :label="p.name" :value="p.name" />
        </el-select>
      </el-form-item>
      <el-form-item :label="t('profiles.content')">
        <el-input v-model="content" type="textarea" :rows="10" readonly class="mono" />
      </el-form-item>
    </el-form>
  </FormDialog>
</template>

<style scoped>
.mono :deep(textarea) {
  font-family: "Consolas", "Menlo", monospace;
}
</style>
