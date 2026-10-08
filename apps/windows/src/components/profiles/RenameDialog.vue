<!-- 重命名配置文件。`target` 为被重命名的当前名称。 -->
<script setup lang="ts">
import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import FormDialog from "@/components/FormDialog.vue";
import { useAction } from "@/composables/useAction";
import { useProfilesStore } from "@/stores/profiles";

const props = defineProps<{ target: string | null }>();
const visible = defineModel<boolean>({ required: true });
const { t } = useI18n();
const profiles = useProfilesStore();
const { busy, run } = useAction();

const newName = ref("");
watch(visible, (v) => {
  if (v) newName.value = props.target ?? "";
});

async function submit() {
  const name = newName.value.trim();
  if (props.target === null || !name) return;
  const target = props.target;
  const r = await run(() => profiles.rename(target, name), { success: true });
  if (r.ok) visible.value = false;
}
</script>

<template>
  <FormDialog
    v-model="visible"
    :title="t('profiles.rename')"
    :submitting="busy"
    :submit-disabled="!newName.trim()"
    @submit="submit"
  >
    <el-form label-position="top" @submit.prevent="submit">
      <el-form-item :label="t('profiles.name')">
        <el-input v-model="newName" />
      </el-form-item>
    </el-form>
  </FormDialog>
</template>
