<!-- src/views/SettingsView.vue - 设置：常规 / 代理 / TUN / 高级 / 关于
     直接绑定 configStore.config 的字段，点"保存"统一提交。
     需要立即实时生效的开关（系统代理 / 代理模式）走统一编排层命令
     （proxyApi.setSystemProxy / setProxyMode），而不是等"保存"。 -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { ElMessage, ElMessageBox } from "element-plus";
import { open } from "@tauri-apps/plugin-dialog";
import { readTextFile } from "@tauri-apps/plugin-fs";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { configApi, type ClashConfig } from "@/api/config";
import type { GeoDataStatus } from "@/api/geodata";
import { geodataApi } from "@/api/geodata";
import { proxyApi } from "@/api/proxy";
import { utilApi } from "@/api/util";
import { changeLocale } from "@/i18n";
import { useAppStore } from "@/stores/app";
import { useConfigStore } from "@/stores/config";
import { useCoreStore } from "@/stores/core";
import { getTheme, setTheme } from "../theme";

const { t } = useI18n();
const appStore = useAppStore();
const configStore = useConfigStore();
const coreStore = useCoreStore();

const cfg = computed(() => configStore.config ?? ({} as ClashConfig));

const theme = ref<"dark" | "light">(getTheme());
watch(theme, (v) => setTheme(v), { immediate: true });

const tunInterface = computed({
  get: () => cfg.value.tun["interface-name"] ?? "",
  set: (v: string) => {
    cfg.value.tun["interface-name"] = v || null;
  },
});

const logLevels = ["debug", "info", "warning", "error"];
// geodata-mode 合法值 = 应用级（manual/use-external/remote，不写给 mihomo）
// + mihomo 语义值（metax/v2ray）。默认值是 manual，选项必须包含它，
// 否则 select 找不到匹配项显示空白（界面状态 ≠ 应用状态）。
const geodataModes = ["manual", "use-external", "remote", "metax", "v2ray"];
// mihomo 官方模板：find-process-mode 只有 always / strict / off 三值。
const findProcessModes = ["off", "strict", "always"];
// mihomo 官方模板仅这三值；script 是 Clash Premium 遗留，后端会拒绝。
const proxyModes = ["rule", "global", "direct"];

const geo = ref<GeoDataStatus | null>(null);
const geoUpdating = ref(false);

const autostart = ref(false);
const autostartLoading = ref(false);

// 托盘触发 geo 更新/回滚、开机自启切换时，本页对应状态要跟随刷新
// （后端 updater/tray 会 emit 事件；前端在此监听，保证跨入口一致）。
let unlisteners: UnlistenFn[] = [];

onMounted(async () => {
  if (!configStore.config) await configStore.load();
  try {
    geo.value = await geodataApi.status();
  } catch {
    geo.value = null;
  }
  try {
    autostart.value = await utilApi.getAutostart();
  } catch {
    autostart.value = false;
  }

  const onGeoChanged = () => {
    void geodataApi.status().then((s) => (geo.value = s)).catch(() => {});
  };
  const onAutostartChanged = (e: { payload: { enable: boolean } }) => {
    autostart.value = e.payload.enable;
  };
  unlisteners = [
    await listen("geodata-updated", onGeoChanged),
    await listen("geodata-rolled-back", onGeoChanged),
    await listen("autostart-changed", onAutostartChanged),
  ];
});

onUnmounted(() => {
  unlisteners.forEach((fn) => fn());
  unlisteners = [];
});

function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** 统一的错误提示文案：复用 i18n 的 common.error 键（zh: 错误 / en: Error）。 */
function errText(e: unknown): string {
  return `${t("common.error")}: ${String(e)}`;
}

async function onSave() {
  try {
    await configStore.save();
    ElMessage.success(t("common.success"));
  } catch (e) {
    ElMessage.error(errText(e));
  }
}

/** 代理模式切换：走编排层（持久化 + PATCH 运行中核心 + 托盘刷新），
 *  成功后同步本地 store 为真实状态。 */
async function onProxyModeChange(val: string) {
  try {
    await proxyApi.setProxyMode(val);
    if (configStore.config) configStore.config.mode = val;
    ElMessage.success(t("common.success"));
  } catch (e) {
    ElMessage.error(String(e));
  }
}

/** 系统代理开关：走编排层（持久化用户意图 + 写注册表真实生效），
 *  成功后同步本地 store，避免下次整包保存时把该字段覆盖回 false。 */
async function onSystemProxyChange(val: boolean) {
  try {
    await proxyApi.setSystemProxy(val);
    if (configStore.config) configStore.config["system-proxy"] = val;
    ElMessage.success(t("common.success"));
  } catch (e) {
    ElMessage.error(String(e));
  }
}

/** 开机自启开关：调用编排层 set_autostart（写注册表 Run 键），
 *  失败回滚开关状态。 */
async function onAutostartChange(val: boolean) {
  autostartLoading.value = true;
  try {
    await utilApi.setAutostart(val);
    ElMessage.success(t("common.success"));
  } catch (e) {
    autostart.value = !val; // 失败回滚
    ElMessage.error(String(e));
  } finally {
    autostartLoading.value = false;
  }
}

async function onLocaleChange(val: string) {
  try {
    await configStore.patch({ locale: val });
    await changeLocale(val);
    ElMessage.success(t("common.success"));
  } catch (e) {
    ElMessage.error(errText(e));
    // 失败回滚内存为后端真实值（patch 失败时内存本就未改，此处为兜底）。
    await configStore.load().catch(() => {});
  }
}

async function onOpenDataDir() {
  try {
    await utilApi.openDataDir();
  } catch (e) {
    ElMessage.error(String(e));
  }
}

async function onReset() {
  try {
    await ElMessageBox.confirm(t("advanced.reset_confirm"), t("common.confirm"), {
      type: "warning",
    });
  } catch {
    return;
  }
  try {
    await configStore.reset();
    ElMessage.success(t("common.success"));
  } catch (e) {
    ElMessage.error(errText(e));
    // 重置失败时从后端恢复当前内存状态。
    await configStore.load().catch(() => {});
  }
}

/** 导入配置：文件对话框选取 .yaml/.yml → 自动读取内容 → 后端解析导入并生效。 */
async function onImportConfig() {
  try {
    const file = await open({
      multiple: false,
      filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
    });
    if (typeof file !== "string") return; // 用户取消
    const content = await readTextFile(file);
    await configApi.import(content);
    await configStore.load();
    ElMessage.success(t("advanced.import_config_done"));
  } catch (e) {
    ElMessage.error(String(e));
  }
}

/** 导出配置：后端自动生成完整 mihomo 配置文件到数据目录，提示路径即可。 */
async function onExportConfig() {
  try {
    const path = await configApi.export();
    ElMessage.success(`${t("advanced.export_config_done")} ${path}`);
  } catch (e) {
    ElMessage.error(String(e));
  }
}

async function onUpdateGeo() {
  geoUpdating.value = true;
  try {
    await geodataApi.update();
    ElMessage.success(t("geodata.done"));
    geo.value = await geodataApi.status();
  } catch (e) {
    ElMessage.error(String(e));
  } finally {
    geoUpdating.value = false;
  }
}
</script>

<template>
  <div v-if="configStore.config" class="page">
    <h2 class="page-title">{{ $t("settings.title") }}</h2>

    <el-tabs tab-position="left" class="settings-tabs">
      <!-- 常规 -->
      <el-tab-pane :label="$t('settings.tabs.general')">
        <el-form label-width="150px" class="settings-form">
          <el-form-item :label="$t('settings.language')">
            <el-select
              :model-value="cfg.locale"
              style="width: 220px"
              @change="onLocaleChange"
            >
              <el-option
                v-for="loc in appStore.locales"
                :key="loc"
                :label="loc"
                :value="loc"
              />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('settings.theme')">
            <el-radio-group v-model="theme">
              <el-radio-button value="dark">{{ $t("settings.theme_dark") }}</el-radio-button>
              <el-radio-button value="light">{{ $t("settings.theme_light") }}</el-radio-button>
            </el-radio-group>
          </el-form-item>
          <el-form-item :label="$t('settings.autostart')">
            <el-switch v-model="autostart" :loading="autostartLoading" @change="onAutostartChange" />
            <span class="autostart-hint">{{ $t("settings.silent_autostart") }}</span>
          </el-form-item>
          <el-form-item :label="$t('general.mixed_port')">
            <el-input-number v-model="cfg['mixed-port']" :min="1" :max="65535" />
          </el-form-item>
          <el-form-item :label="$t('general.allow_lan')">
            <el-switch v-model="cfg['allow-lan']" />
          </el-form-item>
          <el-form-item :label="$t('general.ipv6')">
            <el-switch v-model="cfg.ipv6" />
          </el-form-item>
          <el-form-item :label="$t('general.log_level')">
            <el-select v-model="cfg['log-level']" style="width: 220px">
              <el-option v-for="lv in logLevels" :key="lv" :label="lv" :value="lv" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('general.geodata_mode')">
            <el-select v-model="cfg['geodata-mode']" style="width: 220px">
              <el-option v-for="m in geodataModes" :key="m" :label="m" :value="m" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('general.geo_auto_update')">
            <el-switch v-model="cfg['geo-auto-update']" />
          </el-form-item>
          <el-form-item :label="$t('general.find_process_mode')">
            <el-select v-model="cfg['find-process-mode']" style="width: 220px">
              <el-option
                v-for="m in findProcessModes"
                :key="m"
                :label="m"
                :value="m"
              />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('general.proxy_mode')">
            <el-select
              :model-value="cfg.mode"
              style="width: 220px"
              @change="onProxyModeChange"
            >
              <el-option
                v-for="m in proxyModes"
                :key="m"
                :label="$t('tray.mode_' + m)"
                :value="m"
              />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('general.system_proxy')">
            <el-switch
              :model-value="cfg['system-proxy']"
              @change="onSystemProxyChange"
            />
          </el-form-item>
          <el-form-item :label="$t('settings.data_dir')">
            <el-button @click="onOpenDataDir">{{ $t("settings.open_data_dir") }}</el-button>
          </el-form-item>
          <el-form-item>
            <el-button type="primary" @click="onSave">{{ $t("common.save") }}</el-button>
          </el-form-item>
        </el-form>
      </el-tab-pane>

      <!-- 代理 -->
      <el-tab-pane :label="$t('settings.tabs.proxy')">
        <el-form label-width="150px" class="settings-form">
          <el-form-item :label="$t('proxy.config_mixin')">
            <el-switch v-model="cfg['mixin-enabled']" />
          </el-form-item>
          <el-form-item :label="$t('proxy.tun_mode')">
            <el-switch v-model="cfg.tun.enable" />
          </el-form-item>
          <el-form-item>
            <el-button type="primary" @click="onSave">{{ $t("common.save") }}</el-button>
          </el-form-item>
        </el-form>
      </el-tab-pane>

      <!-- TUN -->
      <el-tab-pane :label="$t('settings.tabs.tun')">
        <el-form label-width="150px" class="settings-form">
          <el-form-item :label="$t('tun.enable')">
            <el-switch v-model="cfg.tun.enable" />
          </el-form-item>
          <el-form-item :label="$t('tun.stack')">
            <el-select v-model="cfg.tun.stack" style="width: 300px">
              <el-option value="system" :label="$t('tun.stack_system')" />
              <el-option value="gvisor" :label="$t('tun.stack_gvisor')" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('tun.auto_route')">
            <el-switch v-model="cfg.tun['auto-route']" />
          </el-form-item>
          <el-form-item :label="$t('tun.auto_detect_interface')">
            <el-switch v-model="cfg.tun['auto-detect-interface']" />
          </el-form-item>
          <el-form-item :label="$t('tun.interface_name')">
            <el-input
              v-model="tunInterface"
              style="width: 300px"
              :placeholder="$t('tun.interface_name')"
            />
          </el-form-item>
          <el-form-item>
            <el-button type="primary" @click="onSave">{{ $t("common.save") }}</el-button>
          </el-form-item>
        </el-form>
      </el-tab-pane>

      <!-- 高级 -->
      <el-tab-pane :label="$t('settings.tabs.advanced')">
        <el-form label-width="150px" class="settings-form">
          <el-form-item :label="$t('advanced.geox_url')">
            <el-input v-model="cfg.advanced['geox-url']" />
          </el-form-item>
          <el-form-item :label="$t('advanced.geoip_url')">
            <el-input v-model="cfg.advanced['geoip-url']" />
          </el-form-item>
          <el-form-item :label="$t('advanced.geosite_url')">
            <el-input v-model="cfg.advanced['geosite-url']" />
          </el-form-item>
          <el-form-item :label="$t('geodata.title')">
            <div class="geo-row">
              <el-button :loading="geoUpdating" @click="onUpdateGeo">
                {{ $t("geodata.update_btn") }}
              </el-button>
              <span v-if="geo" class="geo-status">
                GeoIP: {{ geo.geoip.exists ? fmtSize(geo.geoip.size) : "—" }}
                · GeoSite: {{ geo.geosite.exists ? fmtSize(geo.geosite.size) : "—" }}
              </span>
            </div>
          </el-form-item>
          <el-form-item>
            <el-button type="primary" @click="onSave">{{ $t("common.save") }}</el-button>
          </el-form-item>
          <el-form-item>
            <el-button @click="onImportConfig">{{ $t("advanced.import_config") }}</el-button>
            <el-button @click="onExportConfig">{{ $t("advanced.export_config") }}</el-button>
            <el-button type="danger" plain @click="onReset">
              {{ $t("advanced.reset_config") }}
            </el-button>
          </el-form-item>
        </el-form>
      </el-tab-pane>

      <!-- 关于 -->
      <el-tab-pane :label="$t('settings.tabs.about')">
        <el-descriptions :column="1" border>
          <el-descriptions-item :label="$t('about.version')">
            {{ appStore.version || "—" }}
          </el-descriptions-item>
          <el-descriptions-item :label="$t('about.core_version')">
            {{ coreStore.status.version ?? "—" }}
          </el-descriptions-item>
        </el-descriptions>
      </el-tab-pane>
    </el-tabs>
  </div>
</template>

<style scoped>
.settings-form {
  max-width: 680px;
}

.settings-form .el-form-item {
  margin-bottom: 22px;
}

.settings-form .el-form-item__label {
  color: var(--text-secondary);
  font-weight: 500;
}

.geo-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.geo-status {
  font-size: 12px;
  color: var(--text-tertiary);
  font-variant-numeric: tabular-nums;
}

.autostart-hint {
  margin-left: 10px;
  font-size: 12px;
  color: var(--text-tertiary);
}
</style>
