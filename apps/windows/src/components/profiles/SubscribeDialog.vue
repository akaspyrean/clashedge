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
const urlError = ref<string | null>(null);

// 简单的 URL 格式校验：必须以 http:// 或 https:// 开头，包含主机名
function validateUrl(url: string): string | null {
  const trimmed = url.trim();
  if (!trimmed) return t("profiles.subscribe_url_required");
  try {
    const u = new URL(trimmed);
    if (u.protocol !== "http:" && u.protocol !== "https:") return t("profiles.subscribe_url_protocol");
    if (!u.hostname) return t("profiles.subscribe_url_host");
    return null;
  } catch {
    return t("profiles.subscribe_url_invalid");
  }
}

// 每次打开都从空白表单开始（成功后也清空，避免订阅 token 残留在内存里的输入框）。
watch(visible, (v) => {
  if (v) {
    url.value = name.value = ua.value = "";
    urlError.value = null;
  }
});

async function submit() {
  // 实时校验
  urlError.value = validateUrl(url.value);
  if (urlError.value) return;

  const r = await run(() => profiles.importFromUrl(name.value.trim(), url.value.trim(), ua.value.trim() || undefined), {
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
    :submit-disabled="!url.trim() || !!urlError"
    @submit="submit"
  >
    <el-form label-position="top" @submit.prevent="submit">
      <el-form-item :label="t('profiles.subscribe_url')" :error="urlError">
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
