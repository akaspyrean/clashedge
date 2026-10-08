<!-- 订阅导入：URL + 可选名称 + 可选 User-Agent。 -->
<script setup lang="ts">
import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import FormDialog from "@/components/FormDialog.vue";
import { useAction } from "@/composables/useAction";
import { useProfilesStore } from "@/stores/profiles";

const visible = defineModel<boolean>({ required: true });
const { t } = useI18n();
const profiles = useProfilesStore();
const { busy, run } = useAction();

const url = ref("");
const name = ref("");
const ua = ref("");

// 每次打开都从空白表单开始（成功后也清空，避免订阅 token 残留在内存里的输入框）。
watch(visible, (v) => {
  if (v) url.value = name.value = ua.value = "";
});

async function submit() {
  const u = url.value.trim();
  if (!u) return;
  const r = await run(() => profiles.importFromUrl(name.value.trim(), u, ua.value.trim() || undefined), {
    success: true,
  });
  if (r.ok) visible.value = false;
}
</script>

<template>
  <FormDialog
    v-model="visible"
    :title="t('profiles.subscribe')"
    :submit-label="t('profiles.subscribe')"
    :submitting="busy"
    :submit-disabled="!url.trim()"
    @submit="submit"
  >
    <el-form label-position="top" @submit.prevent="submit">
      <el-form-item :label="t('profiles.subscribe_url')">
        <el-input v-model="url" :placeholder="t('profiles.url_placeholder')" autocomplete="off" />
      </el-form-item>
      <el-form-item :label="t('profiles.name_optional')">
        <el-input v-model="name" :placeholder="t('profiles.name_optional_hint')" />
      </el-form-item>
      <el-form-item :label="t('profiles.subscribe_ua')">
        <el-input v-model="ua" :placeholder="t('profiles.subscribe_ua_hint')" />
      </el-form-item>
    </el-form>
  </FormDialog>
</template>
