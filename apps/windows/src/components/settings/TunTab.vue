<!-- 设置 · TUN：虚拟网卡参数。 -->
<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { ClashConfig } from "@/api/config";
import PrefList from "@/components/PrefList.vue";
import PrefRow from "@/components/PrefRow.vue";
import { useAction } from "@/composables/useAction";
import { useConfigStore } from "@/stores/config";

const { t } = useI18n();
const configStore = useConfigStore();
const cfg = computed(() => configStore.config as ClashConfig);
const tun = useAction();

const interfaceName = computed({
  get: () => cfg.value.tun["interface-name"] ?? "",
  set: (v: string) => {
    cfg.value.tun["interface-name"] = v || null;
  },
});

const onTun = (v: boolean | string | number) =>
  tun.run(() => configStore.setTunMode(Boolean(v)), { success: true });
</script>

<template>
  <PrefList>
    <PrefRow :title="t('tun.enable')">
      <template #default="{ label }">
        <el-switch :model-value="cfg.tun.enable" :aria-label="label" :loading="tun.busy.value" @change="onTun" />
      </template>
    </PrefRow>
    <PrefRow :title="t('tun.stack')">
      <template #default="{ label }">
        <el-select v-model="cfg.tun.stack" :aria-label="label" style="width: 160px">
          <el-option value="mixed" :label="t('tun.stack_mixed')" />
          <el-option value="system" :label="t('tun.stack_system')" />
          <el-option value="gvisor" :label="t('tun.stack_gvisor')" />
        </el-select>
      </template>
    </PrefRow>
    <PrefRow :title="t('tun.auto_route')">
      <template #default="{ label }">
        <el-switch v-model="cfg.tun['auto-route']" :aria-label="label" />
      </template>
    </PrefRow>
    <PrefRow :title="t('tun.auto_detect_interface')">
      <template #default="{ label }">
        <el-switch v-model="cfg.tun['auto-detect-interface']" :aria-label="label" />
      </template>
    </PrefRow>
    <PrefRow :title="t('tun.strict_route')">
      <template #default="{ label }">
        <el-switch v-model="cfg.tun['strict-route']" :aria-label="label" />
      </template>
    </PrefRow>
    <PrefRow :title="t('tun.interface_name')">
      <template #default="{ label }">
        <el-input
          v-model="interfaceName"
          :aria-label="label"
          style="width: 220px"
          :placeholder="t('tun.interface_name')"
          clearable
        />
      </template>
    </PrefRow>
  </PrefList>
</template>
