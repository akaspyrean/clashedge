<!-- 设置 · 常规：高频偏好。低频技术项（日志 / 地理数据 / 进程查找）在「高级」。 -->
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { ClashConfig } from "@/api/config";
import CeSegmented from "@/components/ui/CeSegmented.vue";
import PrefList from "@/components/PrefList.vue";
import PrefRow from "@/components/PrefRow.vue";
import { useAction } from "@/composables/useAction";
import { useConfirm } from "@/composables/useConfirm";
import { useAppStore } from "@/stores/app";
import { useConfigStore } from "@/stores/config";
import { isValidCidr } from "@/utils/format";
import { getTheme, setTheme } from "@/theme";

const { t } = useI18n();
const appStore = useAppStore();
const configStore = useConfigStore();
const confirm = useConfirm();

const cfg = computed(() => configStore.config as ClashConfig);

// 即时生效的开关各自带在途守卫（loading）与统一失败提示。
const autostart = useAction();
const locale = useAction();
const mode = useAction();
const systemProxy = useAction();
const dataDir = useAction();

const theme = ref<"system" | "dark" | "light">(getTheme());
const themeOptions = computed(() => [
  { value: "light" as const, label: t("settings.theme_light") },
  { value: "dark" as const, label: t("settings.theme_dark") },
  { value: "system" as const, label: t("settings.theme_system") },
]);
watch(theme, (v) => setTheme(v), { immediate: true });

// mihomo 官方模板仅这三值；script 是 Clash Premium 遗留，后端会拒绝。
const proxyModes = ["rule", "global", "direct"];

// 语言下拉显示本地化名称，不显示 locale 代码。
const localeNames: Record<string, string> = { "zh-CN": "简体中文", "en-US": "English", en: "English" };
const localeLabel = (loc: string): string => localeNames[loc] ?? loc;

const lanAdvancedOpen = ref<string[]>([]);

/** 局域网 CIDR 白名单：输入框逗号/换行分隔文本 <-> string[]。 */
const lanAllowedIpsText = computed({
  get: () => cfg.value["lan-allowed-ips"]?.join(", ") ?? "",
  set: (v: string) => {
    const list = v
      .split(/[\n,]/)
      .map((s) => s.trim())
      .filter(Boolean);
    cfg.value["lan-allowed-ips"] = list.length ? list : undefined;
  },
});

const lanAllowedIpsWarning = computed(() => {
  const bad = (cfg.value["lan-allowed-ips"] ?? []).filter((e) => !isValidCidr(e));
  return bad.length ? `${t("general.lan_ips_invalid")}: ${bad.join(", ")}` : "";
});

const onAutostart = (v: boolean | string | number) =>
  autostart.run(() => appStore.setAutostart(Boolean(v)), { success: true });
const onLocale = (v: string) => locale.run(() => configStore.setLocale(v), { success: true });
const onMode = (v: string) => mode.run(() => configStore.setProxyMode(v), { success: true });
const onSystemProxy = (v: boolean | string | number) =>
  systemProxy.run(() => configStore.setSystemProxy(Boolean(v)), { success: true });
const onOpenDataDir = () => dataDir.run(() => appStore.openDataDir());

/** 允许局域网：从关到开时先确认安全风险，取消则保持关闭。 */
async function onAllowLan(val: boolean | string | number) {
  if (!val) {
    cfg.value["allow-lan"] = false;
    return;
  }
  if (await confirm(t("general.allow_lan_confirm"))) cfg.value["allow-lan"] = true;
}
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

    <PrefRow :title="t('general.mixed_port')">
      <template #default="{ label }">
        <el-input-number v-model="cfg['mixed-port']" :aria-label="label" :min="1" :max="65535" />
      </template>
    </PrefRow>

    <PrefRow :title="t('general.allow_lan')">
      <template #default="{ label }">
        <el-switch :model-value="cfg['allow-lan']" :aria-label="label" @change="onAllowLan" />
      </template>
    </PrefRow>

    <!-- 高级限制：仅局域网连接开启时才显示 / 可设置 -->
    <el-collapse v-if="cfg['allow-lan']" v-model="lanAdvancedOpen" class="lan-advanced">
      <el-collapse-item :title="t('general.lan_advanced')" name="lan">
        <div class="sub-row">
          <label class="sub-label" for="lan-bind-address">{{ t("general.bind_address") }}</label>
          <el-input
            id="lan-bind-address"
            v-model="cfg['bind-address']"
            :placeholder="t('general.bind_address_placeholder')"
            clearable
          />
        </div>
        <div class="sub-row">
          <label class="sub-label" for="lan-allowed-ips">{{ t("general.lan_allowed_ips") }}</label>
          <el-input
            id="lan-allowed-ips"
            v-model="lanAllowedIpsText"
            type="textarea"
            :rows="2"
            :aria-invalid="lanAllowedIpsWarning ? 'true' : undefined"
            :placeholder="t('general.lan_allowed_ips_placeholder')"
          />
          <span v-if="lanAllowedIpsWarning" class="lan-ips-warning" role="alert">
            {{ lanAllowedIpsWarning }}
          </span>
        </div>
      </el-collapse-item>
    </el-collapse>

    <PrefRow :title="t('general.ipv6')">
      <template #default="{ label }"><el-switch v-model="cfg.ipv6" :aria-label="label" /></template>
    </PrefRow>

    <PrefRow :title="t('general.auto_update_subscription')">
      <template #default="{ label }">
        <el-switch v-model="cfg['auto-update-subscription']" :aria-label="label" />
      </template>
    </PrefRow>

    <PrefRow :title="t('general.auto_check_update')">
      <template #default="{ label }">
        <el-switch v-model="cfg['auto-check-update']" :aria-label="label" />
      </template>
    </PrefRow>

    <PrefRow :title="t('general.proxy_mode')" :hint="t('general.proxy_mode_hint')">
      <template #default="{ label }">
        <el-select
          :model-value="cfg.mode"
          :aria-label="label"
          :disabled="mode.busy.value"
          style="width: 160px"
          @change="onMode"
        >
          <el-option v-for="m in proxyModes" :key="m" :label="t('tray.mode_' + m)" :value="m" />
        </el-select>
      </template>
    </PrefRow>

    <PrefRow :title="t('general.system_proxy')" :hint="t('dashboard.system_proxy_hint')">
      <template #default="{ label }">
        <el-switch
          :model-value="cfg['system-proxy']"
          :aria-label="label"
          :loading="systemProxy.busy.value"
          @change="onSystemProxy"
        />
      </template>
    </PrefRow>

    <PrefRow :title="t('settings.data_dir')">
      <el-button @click="onOpenDataDir">{{ t("settings.open_data_dir") }}</el-button>
    </PrefRow>
  </PrefList>
</template>

<style scoped>
/* 从属块（allow-lan 高级限制）：弱背景 + 标签在上、输入在下。 */
.lan-advanced {
  margin: 0 0 4px;
  border-top: none;
  border-bottom: none;
  background: var(--bg-soft);
  border-radius: var(--r-sm);
}

.sub-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px 0;
}

.sub-label {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-secondary);
}

.lan-ips-warning {
  font-size: 12px;
  color: var(--el-color-warning);
}
</style>
