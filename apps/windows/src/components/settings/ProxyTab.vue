<!-- 设置 · 代理：嗅探与 TUN 总开关。 -->
<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { ClashConfig } from "@/api/config";
import PrefList from "@/components/PrefList.vue";
import PrefRow from "@/components/PrefRow.vue";
import { useAction } from "@/composables/useAction";
import { useSettingsSave } from "@/composables/useSettingsSave";
import { useConfigStore } from "@/stores/config";

const { t } = useI18n();
const configStore = useConfigStore();
const { save } = useSettingsSave();
const cfg = computed(() => configStore.config as ClashConfig);
const tun = useAction();

const onTun = (v: boolean | string | number) =>
  tun.run(() => configStore.setTunMode(Boolean(v)), { success: true });
</script>

<template>
  <PrefList :save-label="t('common.save')" @save="save">
    <PrefRow :title="t('general.sniffer')" :hint="t('general.sniffer_hint')">
      <template #default="{ label }"><el-switch v-model="cfg.sniffer" :aria-label="label" /></template>
    </PrefRow>
    <PrefRow :title="t('proxy.tun_mode')">
      <template #default="{ label }">
        <el-switch :model-value="cfg.tun.enable" :aria-label="label" :loading="tun.busy.value" @change="onTun" />
      </template>
    </PrefRow>
  </PrefList>
</template>
