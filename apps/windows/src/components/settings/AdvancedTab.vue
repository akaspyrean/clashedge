<!-- 设置 · 高级：低频技术项集中在此（日志 / 地理数据 / 进程查找 / URL 覆写）。 -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ClashConfig } from "@/api/config";
import PrefList from "@/components/PrefList.vue";
import PrefRow from "@/components/PrefRow.vue";
import { useNotify } from "@/composables/useNotify";
import { useConfigStore } from "@/stores/config";
import { useGeodataStore } from "@/stores/geodata";
import { formatBytes } from "@/utils/format";

const { t } = useI18n();
const configStore = useConfigStore();
const geoStore = useGeodataStore();
const notify = useNotify();

const cfg = computed(() => configStore.config as ClashConfig);

const logLevels = ["debug", "info", "warning", "error"];
// geodata-mode 合法值 = 应用级（manual/use-external/remote，不写给 mihomo）
// + mihomo 语义值（metax/v2ray）。默认值是 manual，选项必须包含它，
// 否则 select 找不到匹配项显示空白（界面状态 ≠ 应用状态）。
const geodataModes = ["manual", "use-external", "remote", "metax", "v2ray"];
// mihomo 官方模板：find-process-mode 只有 always / strict / off 三值。
const findProcessModes = ["off", "strict", "always"];

const geoSize = (f?: { exists: boolean; size: number }) => (f?.exists ? formatBytes(f.size) : "—");

async function onUpdateGeo() {
  try {
    await geoStore.update();
    notify.ok(t("geodata.done"));
  } catch (e) {
    notify.fail(e);
  }
}

// 托盘触发 geo 更新时，本页的文件大小要跟随刷新。
let unlisten: UnlistenFn | undefined;
onMounted(async () => {
  void geoStore.refresh();
  unlisten = await listen("geodata-updated", () => void geoStore.refresh());
});
onUnmounted(() => unlisten?.());
</script>

<template>
  <PrefList>
    <PrefRow :title="t('general.log_level')">
      <template #default="{ label }">
        <el-select v-model="cfg['log-level']" :aria-label="label" style="width: 160px">
          <el-option v-for="lv in logLevels" :key="lv" :label="lv" :value="lv" />
        </el-select>
      </template>
    </PrefRow>
    <PrefRow :title="t('general.find_process_mode')">
      <template #default="{ label }">
        <el-select v-model="cfg['find-process-mode']" :aria-label="label" style="width: 160px">
          <el-option v-for="m in findProcessModes" :key="m" :label="t('general.fpm_' + m)" :value="m" />
        </el-select>
      </template>
    </PrefRow>
    <PrefRow :title="t('general.geodata_mode')">
      <template #default="{ label }">
        <el-select v-model="cfg['geodata-mode']" :aria-label="label" style="width: 160px">
          <el-option v-for="m in geodataModes" :key="m" :label="t('general.gdm_' + m)" :value="m" />
        </el-select>
      </template>
    </PrefRow>
    <PrefRow :title="t('advanced.geoip_url')" :hint="t('advanced.geo_url_hint')" wide>
      <template #default="{ label }">
        <el-input v-model="cfg.advanced['geoip-url']" :aria-label="label" clearable />
      </template>
    </PrefRow>
    <PrefRow :title="t('advanced.geosite_url')" :hint="t('advanced.geo_url_hint')" wide>
      <template #default="{ label }">
        <el-input v-model="cfg.advanced['geosite-url']" :aria-label="label" clearable />
      </template>
    </PrefRow>
    <PrefRow :title="t('advanced.tcp_concurrent')" :hint="t('advanced.tcp_concurrent_hint')">
      <el-switch v-model="cfg.advanced['tcp-concurrent']" :aria-label="t('advanced.tcp_concurrent')" />
    </PrefRow>
    <PrefRow :title="t('advanced.unified_delay')" :hint="t('advanced.unified_delay_hint')">
      <el-switch v-model="cfg.advanced['unified-delay']" :aria-label="t('advanced.unified_delay')" />
    </PrefRow>
    <PrefRow :title="t('geodata.title')">
      <div class="geo-row">
        <span v-if="geoStore.status" class="geo-status" role="status">
          GeoIP: {{ geoSize(geoStore.status.geoip) }} · GeoSite: {{ geoSize(geoStore.status.geosite) }}
        </span>
        <el-switch
          v-model="cfg['geo-auto-update']"
          :aria-label="t('general.geo_auto_update')"
        />
        <span class="geo-ctl-label">{{ t("geodata.auto_switch") }}</span>
        <el-button :loading="geoStore.updating" @click="onUpdateGeo">
          {{ t("geodata.update_btn") }}
        </el-button>
      </div>
    </PrefRow>
  </PrefList>
</template>

<style scoped>
/* 整组控件右对齐（状态信息也在组内，随组靠右）；窄屏换行。 */
.geo-row {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--ce-space-3);
  flex-wrap: wrap;
}

.geo-status {
  font: var(--ce-type-caption);
  color: var(--ce-text-tertiary);
  font-variant-numeric: tabular-nums;
}

.geo-ctl-label {
  font: var(--ce-type-callout);
  color: var(--ce-text-secondary);
  margin-left: calc(var(--ce-space-2) * -1);
}
</style>
