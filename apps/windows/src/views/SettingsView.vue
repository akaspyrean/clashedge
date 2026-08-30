<!-- src/views/SettingsView.vue - 设置：常规 / 代理 / TUN / 高级 / 关于
     直接绑定 configStore.config 的字段，点"保存"统一提交。
     需要立即实时生效的开关（系统代理 / 代理模式）走统一编排层命令
     （proxyApi.setSystemProxy / setProxyMode），而不是等"保存"。
     布局：偏好行式（标题+副文本 居左、控件 居右），与概览页同一套范式；
     低频技术项（日志级别/地理数据/进程查找）归位「高级」页。 -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { ElMessage, ElMessageBox } from "element-plus";
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

const theme = ref<"system" | "dark" | "light">(getTheme());
watch(theme, (v) => setTheme(v), { immediate: true });

// 响应式 tabs：设置页容器宽 < 700px 时切为顶部布局，避免左置标签挤压内容。
const pageEl = ref<HTMLElement | null>(null);
const compactTabs = ref(false);
let resizeObserver: ResizeObserver | undefined;

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

// 语言下拉显示本地化名称（简体中文 / English），不显示 locale 代码。
const localeNames: Record<string, string> = {
  "zh-CN": "简体中文",
  "en-US": "English",
  en: "English",
};
const localeLabel = (loc: string): string => localeNames[loc] ?? loc;

const geo = ref<GeoDataStatus | null>(null);
const geoUpdating = ref(false);

const autostartLoading = ref(false);

const tunLoading = ref(false);

/** P0 降级模式：用户确认备份位置并同意覆盖损坏 config.yaml 后，保存才放行
 * （后端同样二次拦截未确认的保存）。 */
const degradedConfirmed = ref(false);

const allowLanAdvancedOpen = ref<string[]>([]);

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

/** 轻量 CIDR 形态校验：IPv4 x.x.x.x/n（0-32）或含 ":" 的 IPv6 前缀。 */
function isValidCidr(entry: string): boolean {
  if (entry.includes(":")) return true; // IPv6 前缀，仅做形态放行
  const m = entry.match(/^(\d{1,3}(?:\.\d{1,3}){3})\/(\d{1,2})$/);
  if (!m) return false;
  if (Number(m[2]) > 32) return false;
  return m[1].split(".").every((oct) => Number(oct) <= 255);
}

const lanAllowedIpsWarning = computed(() => {
  const bad = (cfg.value["lan-allowed-ips"] ?? []).filter((e) => !isValidCidr(e));
  return bad.length ? `${t("general.lan_ips_invalid")}: ${bad.join(", ")}` : "";
});

// 托盘触发 geo 更新、开机自启切换时，本页对应状态要跟随刷新
// （后端 updater/tray 会 emit 事件；前端在此监听，保证跨入口一致）。
let unlisteners: UnlistenFn[] = [];

onMounted(async () => {
  if (!configStore.config) await configStore.load();
  // P0 降级模式：损坏配置下提示用户备份位置（独立于 config 本体）。
  await configStore.loadDegradedInfo();
  try {
    geo.value = await geodataApi.status();
  } catch {
    geo.value = null;
  }
  try {
    appStore.autostart = await utilApi.getAutostart();
  } catch {
    appStore.autostart = false;
  }

  const onGeoChanged = () => {
    void geodataApi.status().then((s) => (geo.value = s)).catch(() => {});
  };
  // 自启事件改由 main.ts 生命周期级监听写入共享 app store（避免页面级不同步）。
  // 本页 geodata 更新仍为页面级监听（仅设置页关注）。
  unlisteners = [
    await listen("geodata-updated", onGeoChanged),
  ];

  // 容器宽度驱动 tabs 布局（ResizeObserver，无需第三方库）。
  resizeObserver = new ResizeObserver((entries) => {
    compactTabs.value = entries[0].contentRect.width < 700;
  });
  if (pageEl.value) resizeObserver.observe(pageEl.value);
});

onUnmounted(() => {
  unlisteners.forEach((fn) => fn());
  unlisteners = [];
  resizeObserver?.disconnect();
  resizeObserver = undefined;
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
    // P0：降级模式下普通保存必须带用户确认（勾选降级横幅中的复选框）。
    // 后端在未确认时同样拒绝，前端这里提前给可操作的错误提示。
    if (configStore.degraded && !degradedConfirmed.value) {
      ElMessage.error(t("settings.degraded_save_blocked"));
      return;
    }
    await configStore.save(degradedConfirmed.value);
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
    appStore.autostart = val;
    ElMessage.success(t("common.success"));
  } catch (e) {
    appStore.autostart = !val; // 失败回滚
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

/** TUN 开关（代理 Tab 与 TUN Tab 共用）：走编排层 set_tun_mode
 *  （持久化 → 重写 runtime → PATCH 运行中核心 → 失败重启回退），
 *  成功后同步本地 store；不再直接改配置走整包保存。 */
async function onTunEnableChange(val: boolean) {
  tunLoading.value = true;
  try {
    await proxyApi.setTunMode(val);
    if (configStore.config) configStore.config.tun.enable = val;
    ElMessage.success(t("common.success"));
  } catch (e) {
    ElMessage.error(String(e));
    // 编排命令失败时后端会回退，本地从后端恢复真实状态。
    await configStore.load().catch(() => {});
  } finally {
    tunLoading.value = false;
  }
}

/** 允许局域网开关：从关到开时先确认安全风险，取消则保持关闭。 */
async function onAllowLanChange(val: boolean | string | number) {
  const enable = Boolean(val);
  if (!enable) {
    cfg.value["allow-lan"] = false;
    return;
  }
  try {
    await ElMessageBox.confirm(t("general.allow_lan_confirm"), t("common.confirm"), {
      type: "warning",
    });
  } catch {
    return; // 取消：UI 保持关闭
  }
  cfg.value["allow-lan"] = true;
}

/** 导入配置：文件选择与读取全部收口到后端（pick_import_file：Rust 侧弹
 *  系统对话框 + 校验扩展名与大小上限），前端不再接触任意绝对路径；
 *  取消选择返回 null，解析导入并生效走 import_config。 */
async function onImportConfig() {
  try {
    const content = await configApi.pickImportFile();
    if (content === null) return; // 用户取消
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
  <div v-if="configStore.config" ref="pageEl" class="page">
    <h2 class="page-title">{{ $t("settings.title") }}</h2>

    <!-- P0 降级模式横幅：config.yaml 损坏时明确告知用户并阻止无确认保存 -->
    <el-alert
      v-if="configStore.degraded"
      type="warning"
      show-icon
      :closable="false"
      class="degraded-alert"
      :title="configStore.degradedMessage"
    >
      <template #default>
        <div class="degraded-body">
          <div>
            {{
              configStore.degradedBackupFile
                ? $t("settings.degraded_banner", { path: configStore.degradedBackupFile })
                : $t("settings.degraded_backup_fallback")
            }}
          </div>
          <el-checkbox v-model="degradedConfirmed">
            {{ $t("settings.degraded_confirm") }}
          </el-checkbox>
        </div>
      </template>
    </el-alert>

    <el-tabs :tab-position="compactTabs ? 'top' : 'left'" class="settings-tabs">
      <!-- 常规：高频偏好。低频技术项（日志/地理数据/进程查找）在「高级」。 -->
      <el-tab-pane :label="$t('settings.tabs.general')">
        <div class="pref-list">
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("settings.language") }}</div>
            </div>
            <div class="pref-control">
              <el-select :model-value="cfg.locale" style="width: 160px" @change="onLocaleChange">
                <el-option v-for="loc in appStore.locales" :key="loc" :label="localeLabel(loc)" :value="loc" />
              </el-select>
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("settings.theme") }}</div>
            </div>
            <div class="pref-control">
              <el-radio-group v-model="theme">
                <el-radio-button value="system">{{ $t("settings.theme_system") }}</el-radio-button>
                <el-radio-button value="light">{{ $t("settings.theme_light") }}</el-radio-button>
                <el-radio-button value="dark">{{ $t("settings.theme_dark") }}</el-radio-button>
              </el-radio-group>
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("settings.autostart") }}</div>
              <div class="pref-hint">{{ $t("settings.silent_autostart") }}</div>
            </div>
            <div class="pref-control">
              <el-switch v-model="appStore.autostart" :loading="autostartLoading" @change="onAutostartChange" />
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("general.mixed_port") }}</div>
            </div>
            <div class="pref-control">
              <el-input-number v-model="cfg['mixed-port']" :min="1" :max="65535" />
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("general.allow_lan") }}</div>
            </div>
            <div class="pref-control">
              <el-switch :model-value="cfg['allow-lan']" @change="onAllowLanChange" />
            </div>
          </div>
          <!-- 高级限制：仅局域网连接开启时才显示/可设置 -->
          <el-collapse v-if="cfg['allow-lan']" v-model="allowLanAdvancedOpen" class="lan-advanced">
            <el-collapse-item :title="$t('general.lan_advanced')" name="lan">
              <div class="sub-row">
                <div class="sub-label">{{ $t("general.bind_address") }}</div>
                <el-input
                  v-model="cfg['bind-address']"
                  :placeholder="$t('general.bind_address_placeholder')"
                  clearable
                />
              </div>
              <div class="sub-row">
                <div class="sub-label">{{ $t("general.lan_allowed_ips") }}</div>
                <el-input
                  v-model="lanAllowedIpsText"
                  type="textarea"
                  :rows="2"
                  :placeholder="$t('general.lan_allowed_ips_placeholder')"
                />
                <span v-if="lanAllowedIpsWarning" class="lan-ips-warning">
                  {{ lanAllowedIpsWarning }}
                </span>
              </div>
            </el-collapse-item>
          </el-collapse>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("general.ipv6") }}</div>
            </div>
            <div class="pref-control">
              <el-switch v-model="cfg.ipv6" />
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("general.auto_update_subscription") }}</div>
            </div>
            <div class="pref-control">
              <el-switch v-model="cfg['auto-update-subscription']" />
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("general.proxy_mode") }}</div>
              <div class="pref-hint">{{ $t("general.proxy_mode_hint") }}</div>
            </div>
            <div class="pref-control">
              <el-select :model-value="cfg.mode" style="width: 160px" @change="onProxyModeChange">
                <el-option v-for="m in proxyModes" :key="m" :label="$t('tray.mode_' + m)" :value="m" />
              </el-select>
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("general.system_proxy") }}</div>
              <div class="pref-hint">{{ $t("dashboard.system_proxy_hint") }}</div>
            </div>
            <div class="pref-control">
              <el-switch :model-value="cfg['system-proxy']" @change="onSystemProxyChange" />
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("settings.data_dir") }}</div>
            </div>
            <div class="pref-control">
              <el-button @click="onOpenDataDir">{{ $t("settings.open_data_dir") }}</el-button>
            </div>
          </div>
          <div class="pref-save-row">
            <el-button type="primary" @click="onSave">{{ $t("common.save") }}</el-button>
          </div>
        </div>
      </el-tab-pane>

      <!-- 代理 -->
      <el-tab-pane :label="$t('settings.tabs.proxy')">
        <div class="pref-list">
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("proxy.config_mixin") }}</div>
            </div>
            <div class="pref-control">
              <el-switch v-model="cfg['mixin-enabled']" />
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("proxy.tun_mode") }}</div>
            </div>
            <div class="pref-control">
              <el-switch
                :model-value="cfg.tun.enable"
                :loading="tunLoading"
                @change="onTunEnableChange"
              />
            </div>
          </div>
          <div class="pref-save-row">
            <el-button type="primary" @click="onSave">{{ $t("common.save") }}</el-button>
          </div>
        </div>
      </el-tab-pane>

      <!-- TUN -->
      <el-tab-pane :label="$t('settings.tabs.tun')">
        <div class="pref-list">
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("tun.enable") }}</div>
            </div>
            <div class="pref-control">
              <el-switch
                :model-value="cfg.tun.enable"
                :loading="tunLoading"
                @change="onTunEnableChange"
              />
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("tun.stack") }}</div>
            </div>
            <div class="pref-control">
              <el-select v-model="cfg.tun.stack" style="width: 160px">
                <el-option value="mixed" :label="$t('tun.stack_mixed')" />
                <el-option value="system" :label="$t('tun.stack_system')" />
                <el-option value="gvisor" :label="$t('tun.stack_gvisor')" />
              </el-select>
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("tun.auto_route") }}</div>
            </div>
            <div class="pref-control">
              <el-switch v-model="cfg.tun['auto-route']" />
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("tun.auto_detect_interface") }}</div>
            </div>
            <div class="pref-control">
              <el-switch v-model="cfg.tun['auto-detect-interface']" />
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("tun.interface_name") }}</div>
            </div>
            <div class="pref-control">
              <el-input
                v-model="tunInterface"
                style="width: 220px"
                :placeholder="$t('tun.interface_name')"
                clearable
              />
            </div>
          </div>
          <div class="pref-save-row">
            <el-button type="primary" @click="onSave">{{ $t("common.save") }}</el-button>
          </div>
        </div>
      </el-tab-pane>

      <!-- 高级：低频技术项集中在此（日志/地理数据/进程查找/URL 覆写/导入导出） -->
      <el-tab-pane :label="$t('settings.tabs.advanced')">
        <div class="pref-list">
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("general.log_level") }}</div>
            </div>
            <div class="pref-control">
              <el-select v-model="cfg['log-level']" style="width: 160px">
                <el-option v-for="lv in logLevels" :key="lv" :label="lv" :value="lv" />
              </el-select>
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("general.geodata_mode") }}</div>
            </div>
            <div class="pref-control">
              <el-select v-model="cfg['geodata-mode']" style="width: 160px">
                <el-option v-for="m in geodataModes" :key="m" :label="m" :value="m" />
              </el-select>
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("general.geo_auto_update") }}</div>
            </div>
            <div class="pref-control">
              <el-switch v-model="cfg['geo-auto-update']" />
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("general.find_process_mode") }}</div>
            </div>
            <div class="pref-control">
              <el-select v-model="cfg['find-process-mode']" style="width: 160px">
                <el-option v-for="m in findProcessModes" :key="m" :label="m" :value="m" />
              </el-select>
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("advanced.geox_url") }}</div>
            </div>
            <div class="pref-control pref-control-wide">
              <el-input v-model="cfg.advanced['geox-url']" clearable />
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("advanced.geoip_url") }}</div>
            </div>
            <div class="pref-control pref-control-wide">
              <el-input v-model="cfg.advanced['geoip-url']" clearable />
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("advanced.geosite_url") }}</div>
            </div>
            <div class="pref-control pref-control-wide">
              <el-input v-model="cfg.advanced['geosite-url']" clearable />
            </div>
          </div>
          <div class="pref-row">
            <div class="pref-info">
              <div class="pref-title">{{ $t("geodata.title") }}</div>
            </div>
            <div class="pref-control">
              <div class="geo-row">
                <el-button :loading="geoUpdating" @click="onUpdateGeo">
                  {{ $t("geodata.update_btn") }}
                </el-button>
                <span v-if="geo" class="geo-status">
                  GeoIP: {{ geo.geoip.exists ? fmtSize(geo.geoip.size) : "—" }}
                  · GeoSite: {{ geo.geosite.exists ? fmtSize(geo.geosite.size) : "—" }}
                </span>
              </div>
            </div>
          </div>
          <div class="pref-save-row">
            <el-button type="primary" @click="onSave">{{ $t("common.save") }}</el-button>
          </div>
          <div class="pref-danger-row">
            <el-button @click="onImportConfig">{{ $t("advanced.import_config") }}</el-button>
            <el-button @click="onExportConfig">{{ $t("advanced.export_config") }}</el-button>
            <el-button type="danger" plain @click="onReset">
              {{ $t("advanced.reset_config") }}
            </el-button>
          </div>
        </div>
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
.degraded-alert {
  margin-bottom: 14px;
  max-width: 680px;
}

.degraded-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
  align-items: flex-start;
  overflow-wrap: anywhere;
}

/* ---- 偏好行式列表（与概览页 set-row 同范式）----
 * 标题(+副文本) 居左、控件 居右，细分隔线分行；去"表单"感。 */
.pref-list {
  max-width: 680px;
}

.pref-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 12px 0;
  min-height: 44px;
}

.pref-row + .pref-row {
  border-top: 1px solid var(--card-border);
}

.pref-info {
  min-width: 0;
}

.pref-title {
  font-size: 14px;
  font-weight: 500;
  color: var(--text-primary);
}

.pref-hint {
  margin-top: 2px;
  font-size: 12px;
  color: var(--text-tertiary);
}

.pref-control {
  flex: none;
  display: flex;
  justify-content: flex-end;
}

/* 宽控件（URL 覆写）：吃满行内剩余宽度。 */
.pref-control-wide {
  flex: 1;
  min-width: 0;
}

.pref-control-wide .el-input {
  width: 100%;
}

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

/* 保存行：右对齐，与列表体隔开。 */
.pref-save-row {
  display: flex;
  justify-content: flex-end;
  padding-top: 16px;
}

/* 危险操作行：底部独立区域，顶部细分隔线。 */
.pref-danger-row {
  display: flex;
  justify-content: flex-end;
  gap: 0;
  margin-top: 8px;
  padding-top: 16px;
  border-top: 1px solid var(--card-border);
}

.geo-row {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}

.geo-status {
  font-size: 12px;
  color: var(--text-tertiary);
  font-variant-numeric: tabular-nums;
}
</style>
