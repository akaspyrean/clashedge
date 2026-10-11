<!-- 设置 · 常规：应用本身的高频偏好（语言 / 主题 / 自启动 / 自动更新订阅 / 数据目录）。
     网络与代理行为在「代理」，应用更新在「关于」，低频技术项在「高级」。 -->
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { ClashConfig } from "@/api/config";
import CeSegmented from "@/components/ui/CeSegmented.vue";
import PrefList from "@/components/PrefList.vue";
import PrefRow from "@/components/PrefRow.vue";
import { useAction } from "@/composables/useAction";
import { useAppStore } from "@/stores/app";
import { useConfigStore } from "@/stores/config";
import { getTheme, setTheme } from "@/theme";

const { t } = useI18n();
const appStore = useAppStore();
const configStore = useConfigStore();

const cfg = computed(() => configStore.config as ClashConfig);

// 即时生效的开关各自带在途守卫（loading）与统一失败提示。
const autostart = useAction();
const locale = useAction();
const dataDir = useAction();

const theme = ref<"system" | "dark" | "light">(getTheme());
const themeOptions = computed(() => [
  { value: "system" as const, label: t("settings.theme_system") },
  { value: "light" as const, label: t("settings.theme_light") },
  { value: "dark" as const, label: t("settings.theme_dark") },
]);
watch(theme, (v) => setTheme(v), { immediate: true });

// 语言下拉显示本地化名称，不显示 locale 代码。
const localeNames: Record<string, string> = { "zh-CN": "简体中文", "en-US": "English", en: "English" };
const localeLabel = (loc: string): string => localeNames[loc] ?? loc;

const onAutostart = (v: boolean | string | number) =>
  autostart.run(() => appStore.setAutostart(Boolean(v)), { success: true });
const onLocale = (v: string) => locale.run(() => configStore.setLocale(v), { success: true });
const onOpenDataDir = () => dataDir.run(() => appStore.openDataDir());
</script>

<template>
  <PrefList>
    <PrefRow :title="t('settings.language')">
      <template #default="{ label }">
        <el-select
          :model-value="cfg.locale"
          :aria-label="label"
          :disabled="locale.busy.value"
          style="width: 160px"
          @change="onLocale"
        >
          <el-option v-for="loc in appStore.locales" :key="loc" :label="localeLabel(loc)" :value="loc" />
        </el-select>
      </template>
    </PrefRow>

    <PrefRow :title="t('settings.theme')">
      <template #default="{ label }">
        <CeSegmented v-model="theme" :options="themeOptions" :ariaLabel="label" />
      </template>
    </PrefRow>

    <PrefRow :title="t('settings.autostart')" :hint="t('settings.silent_autostart')">
      <template #default="{ label }">
        <el-switch
          :model-value="appStore.autostart"
          :aria-label="label"
          :loading="autostart.busy.value"
          @change="onAutostart"
        />
      </template>
    </PrefRow>

    <PrefRow :title="t('general.auto_update_subscription')" :hint="t('general.auto_update_subscription_hint')">
      <template #default="{ label }">
        <el-switch v-model="cfg['auto-update-subscription']" :aria-label="label" />
      </template>
    </PrefRow>

    <PrefRow :title="t('settings.data_dir')">
      <el-button @click="onOpenDataDir">{{ t("settings.open_data_dir") }}</el-button>
    </PrefRow>
  </PrefList>
</template>

