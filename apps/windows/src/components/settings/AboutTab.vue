<!-- 设置 · 关于：版本信息、应用更新（签名清单 → 下载校验暂存 → 启动器安装，仅内置公钥的构建显示）。 -->
<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import PrefList from "@/components/PrefList.vue";
import PrefRow from "@/components/PrefRow.vue";
import { useAction } from "@/composables/useAction";
import { useConfirm } from "@/composables/useConfirm";
import { useNotify } from "@/composables/useNotify";
import { useAppStore } from "@/stores/app";
import { useCoreStore } from "@/stores/core";
import { useUpdateStore } from "@/stores/update";
import { friendlyError } from "@/errors";

const { t } = useI18n();
const appStore = useAppStore();
const coreStore = useCoreStore();
const update = useUpdateStore();
const confirm = useConfirm();
const notify = useNotify();

const discarding = useAction();

/** 检查更新。silent=true（由后台事件触发）时不弹成功 / 失败提示。 */
async function onCheck(silent = false) {
  try {
    const s = await update.check();
    if (!silent && s.status === "up_to_date") notify.ok(t("about.up_to_date"));
  } catch (e) {
    if (!silent) notify.error(t("about.update_failed", { error: friendlyError(e) }));
  }
}

async function onDownload() {
  try {
    const staged = await update.download();
    notify.ok(t("about.staged_ready", { version: staged.version }));
  } catch (e) {
    notify.error(t("about.update_failed", { error: friendlyError(e) }));
  }
}

async function onDiscard() {
  await discarding.run(() => update.discard(), { success: t("about.discarded_staged") });
}

async function onRestartApply() {
  if (!(await confirm(t("about.restart_apply_confirm")))) return;
  try {
    await update.restartAndApply(); // 成功时本进程即将退出
  } catch (e) {
    notify.fail(e);
  }
}

// 启动后的静默检查发现新版本：刷新本页更新状态（后端已缓存验签材料）。
let unlisten: UnlistenFn | undefined;
onMounted(async () => {
  void update.loadStaged();
  void update.loadChainReady();
  unlisten = await listen("update-available", () => void onCheck(true));
});
onUnmounted(() => unlisten?.());
</script>

<template>
  <PrefList>
    <PrefRow :title="t('about.version')">
      <span class="about-value">{{ appStore.version || "—" }}</span>
    </PrefRow>
    <PrefRow :title="t('about.core_version')">
      <span class="about-value">{{ coreStore.status.version ?? "—" }}</span>
    </PrefRow>
    <!-- 仅内置了更新公钥的构建显示；开发构建整行隐藏 -->
    <PrefRow v-if="update.chainReady" :title="t('about.check_update')" :hint="t('about.check_update_hint')">
      <template #info>
        <div v-if="update.availableVersion" class="pref-hint" role="status">
          {{ t("about.new_version", { version: update.availableVersion }) }}
        </div>
        <div v-else-if="update.upToDate" class="pref-hint" role="status">{{ t("about.up_to_date") }}</div>
        <div v-if="update.staged" class="pref-hint" role="status">
          {{ t("about.staged_ready", { version: update.staged.version }) }}
          · {{ t("about.update_staged", { version: update.staged.version }) }}
        </div>
      </template>
      <el-button :loading="update.checking" @click="onCheck(false)">
        {{ update.checking ? t("about.checking") : t("about.check_now") }}
      </el-button>
      <el-button
        v-if="update.availableVersion && !update.staged"
        :loading="update.downloading"
        @click="onDownload"
      >
        {{ update.downloading ? t("about.downloading") : t("about.download_btn") }}
      </el-button>
      <el-button v-if="update.staged" @click="onRestartApply">
        {{ t("about.restart_apply") }}
      </el-button>
      <el-button v-if="update.staged" plain :loading="discarding.busy.value" @click="onDiscard">
        {{ t("about.discard_staged") }}
      </el-button>
    </PrefRow>
  </PrefList>
</template>

<style scoped>
.about-value {
  font: var(--ce-type-body);
  color: var(--ce-text-secondary);
  font-variant-numeric: tabular-nums;
}

.pref-hint {
  margin-top: var(--ce-space-1);
  font: var(--ce-type-caption);
  color: var(--ce-text-tertiary);
}
</style>
