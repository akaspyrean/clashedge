<!-- 设置 · 代理：代理行为与网络参数（模式 / 系统代理 / 端口 / 局域网 / 嗅探）。
     TUN 总开关与虚拟网卡参数在「TUN」分组，不在本组重复。 -->
<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { ClashConfig } from "@/api/config";
import PrefList from "@/components/PrefList.vue";
import PrefRow from "@/components/PrefRow.vue";
import { useAction } from "@/composables/useAction";
import { useConfirm } from "@/composables/useConfirm";
import { useConfigStore } from "@/stores/config";
import { isValidCidr } from "@/utils/format";

const { t } = useI18n();
const configStore = useConfigStore();
const cfg = computed(() => configStore.config as ClashConfig);
const confirm = useConfirm();

// 即时生效的开关各自带在途守卫（loading）与统一失败提示。
const mode = useAction();
const systemProxy = useAction();

// mihomo 官方模板仅这三值；script 是 Clash Premium 遗留，后端会拒绝。
const proxyModes = ["rule", "global", "direct"];

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

const onMode = (v: string) => mode.run(() => configStore.setProxyMode(v), { success: true });
const onSystemProxy = (v: boolean | string | number) =>
  systemProxy.run(() => configStore.setSystemProxy(Boolean(v)), { success: true });

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

    <PrefRow :title="t('general.sniffer')" :hint="t('general.sniffer_hint')">
      <template #default="{ label }"><el-switch v-model="cfg.sniffer" :aria-label="label" /></template>
    </PrefRow>
  </PrefList>
</template>

<style scoped>
/* 从属块（allow-lan 高级限制）：弱背景 + 标签在上、输入在下。 */
.lan-advanced {
  margin: 0 0 var(--ce-space-1);
  border-top: none;
  border-bottom: none;
  background: var(--ce-fill-soft);
  border-radius: var(--ce-radius-md);
}

.sub-row {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-2);
  padding: var(--ce-space-2) 0;
}

.sub-label {
  font: var(--ce-type-callout);
  font-weight: 500;
  color: var(--ce-text-secondary);
}

.lan-ips-warning {
  font: var(--ce-type-caption);
  color: var(--ce-warning-fg);
}
</style>
