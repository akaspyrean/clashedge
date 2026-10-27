<!-- 设置 · TUN：虚拟网卡参数。 -->
<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { ClashConfig } from "@/api/config";
import PrefList from "@/components/PrefList.vue";
import PrefRow from "@/components/PrefRow.vue";
import { useAction } from "@/composables/useAction";
import { useTunElevation } from "@/composables/useTunElevation";
import { useConfigStore } from "@/stores/config";

const { t } = useI18n();
const configStore = useConfigStore();
const cfg = computed(() => configStore.config as ClashConfig);
const tun = useAction();
const elevation = useTunElevation();

const interfaceName = computed({
  get: () => cfg.value.tun["interface-name"] ?? "",
  set: (v: string) => {
    cfg.value.tun["interface-name"] = v || null;
  },
});

const onTun = (v: boolean | string | number) => {
  const on = Boolean(v);
  return tun.run(() => configStore.setTunMode(on), {
    success: on ? t("dashboard.tun_enabled") : t("dashboard.tun_disabled"),
    // 非管理员开启 → 权限预检错误 → 弹「以管理员身份重启」确认，同意则走 UAC 提权
    onError: on ? elevation.handleError : undefined,
  });
};
</script>

<template>
  <PrefList>
    <PrefRow :title="t('tun.enable')" :hint="t('tun.enable_hint')">
      <template #default="{ label }">
        <el-switch :model-value="cfg.tun.enable" :aria-label="label" :loading="tun.busy.value" @change="onTun" />
      </template>
    </PrefRow>
    <PrefRow :title="t('tun.stack')" wide>
      <template #default="{ label }">
        <!-- 宽度随行自适应（上限 360px）：中英最长选项都能完整显示，窄屏自动收缩 -->
        <el-select v-model="cfg.tun.stack" :aria-label="label" style="width: 100%; max-width: 360px">
          <el-option value="mixed" :label="t('tun.stack_mixed')" />
          <el-option value="system" :label="t('tun.stack_system')" />
          <el-option value="gvisor" :label="t('tun.stack_gvisor')" />
        </el-select>
      </template>
    </PrefRow>
    <PrefRow :title="t('tun.auto_route')" :hint="t('tun.auto_route_hint')">
      <template #default="{ label }">
        <el-switch v-model="cfg.tun['auto-route']" :aria-label="label" />
      </template>
    </PrefRow>
    <PrefRow :title="t('tun.auto_detect_interface')" :hint="t('tun.auto_detect_interface_hint')">
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
          :placeholder="t('tun.interface_name_placeholder')"
          clearable
        />
      </template>
    </PrefRow>
  </PrefList>
</template>
