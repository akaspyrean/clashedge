<!-- src/components/DegradedBanner.vue - config.yaml 损坏降级横幅（应用壳顶部，全页可见）。
     降级期间后端拒绝一切会覆盖损坏原文件的写入（设置保存 / 托盘开关 / 切换配置 ……），
     用户在此一次性确认"覆盖并继续"后恢复正常。取代了设置页里的复选框确认。 -->
<script setup lang="ts">
import { onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { useConfigStore } from "@/stores/config";
import { useAction } from "@/composables/useAction";
import StatusBanner from "./StatusBanner.vue";

const { t } = useI18n();
const config = useConfigStore();
const { busy, run } = useAction();

onMounted(() => {
  void config.loadDegradedInfo();
});

async function onConfirm() {
  await run(() => config.confirmOverwriteCorrupt(), { success: t("degraded.confirmed") });
}
</script>

<template>
  <StatusBanner v-if="config.degraded" :title="t('degraded.title')" type="warning">
    <div class="degraded-body">
      <div v-if="config.degradedBackupFile">
        {{ t("degraded.backup", { path: config.degradedBackupFile }) }}
      </div>
      <div>{{ t("degraded.hint") }}</div>
      <el-button size="small" type="warning" :loading="busy" @click="onConfirm">
        {{ t("degraded.confirm") }}
      </el-button>
    </div>
  </StatusBanner>
</template>

<style scoped>
.degraded-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
  align-items: flex-start;
}
</style>
