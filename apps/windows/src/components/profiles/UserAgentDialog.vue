<!-- 单个订阅的自定义 User-Agent（审计 B7：按订阅覆盖拉取 UA；留空 = 内置默认）。 -->
<script setup lang="ts">
import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import FormDialog from "@/components/FormDialog.vue";
import { useAction } from "@/composables/useAction";
import { useProfilesStore } from "@/stores/profiles";

const props = defineProps<{ target: string | null; current?: string | null }>();
const visible = defineModel<boolean>({ required: true });
const { t } = useI18n();
const profiles = useProfilesStore();
const { busy, run } = useAction();

const value = ref("");
watch(visible, (v) => {
  if (v) value.value = props.current ?? "";
});

async function submit() {
  if (props.target === null) return;
  const target = props.target;
  const r = await run(() => profiles.setUserAgent(target, value.value.trim() || null), { success: true });
  if (r.ok) visible.value = false;
}
</script>

<template>
  <FormDialog v-model="visible" :title="t('profiles.subscribe_ua')" :submitting="busy" @submit="submit">
    <el-form label-position="top" @submit.prevent="submit">
      <el-form-item :label="t('profiles.subscribe_ua')">
        <el-input v-model="value" :placeholder="t('profiles.subscribe_ua_hint')" />
      </el-form-item>
    </el-form>
  </FormDialog>
</template>
