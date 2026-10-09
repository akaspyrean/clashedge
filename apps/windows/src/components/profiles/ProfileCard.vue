<!-- 订阅卡片：名称 + 状态标签 / 订阅地址 / 用量进度条 / 一个主操作 + 「⋯」菜单。
     主操作：未使用 → 使用；使用中的订阅 → 更新；使用中的本地配置 → 原始编辑。
     其余动作（重命名、原始编辑、User-Agent、导出、删除，以及未使用订阅的更新）收进菜单。
     只发事件，不碰 store——业务由 ProfilesView 编排。 -->
<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { MoreHorizontal } from "lucide-vue-next";
import type { ProfileInfo } from "@/api/profiles";
import { formatBytes, parseUserinfo } from "@/utils/format";

const props = defineProps<{
  profile: ProfileInfo;
  busy?: boolean;
  /** 本次会话内上一次更新失败 */
  failed?: boolean;
}>();
const emit = defineEmits<{
  (e: "activate"): void;
  (e: "refresh"): void;
  (e: "rename"): void;
  (e: "edit"): void;
  (e: "ua"): void;
  (e: "export"): void;
  (e: "delete"): void;
}>();

const { t } = useI18n();
const EXPIRING_DAYS = 7;

const info = computed(() => parseUserinfo(props.profile.userinfo));
const pct = computed(() =>
  info.value && info.value.total > 0 ? Math.min(100, Math.round((info.value.used / info.value.total) * 100)) : null,
);
const barTone = computed(() => (pct.value === null ? "" : pct.value >= 90 ? "is-danger" : pct.value >= 75 ? "is-warning" : ""));
const usage = computed(() => {
  if (!info.value) return "";
  const total = info.value.total ? formatBytes(info.value.total) : t("profiles.traffic_unlimited");
  return `${formatBytes(info.value.used)} / ${total}`;
});

const daysLeft = computed(() =>
  info.value?.expire ? Math.ceil((info.value.expire * 1000 - Date.now()) / 86_400_000) : null,
);
const expireText = computed(() => {
  if (!info.value) return "";
  if (info.value.expire) return `${t("profiles.traffic_expire")} ${new Date(info.value.expire * 1000).toLocaleDateString()}`;
  return info.value.expire === 0 ? t("profiles.traffic_never") : "";
});

/** 状态标签：颜色 + 文字（soft 底 + fg 字，不用实心底）。 */
const tags = computed(() => {
  const out: { text: string; tone: string }[] = [];
  if (props.profile.active) out.push({ text: t("profiles.active"), tone: "accent" });
  if (props.failed) out.push({ text: t("profiles.status_failed"), tone: "danger" });
  if (daysLeft.value !== null) {
    if (daysLeft.value < 0) out.push({ text: t("profiles.status_expired"), tone: "danger" });
    else if (daysLeft.value <= EXPIRING_DAYS) out.push({ text: t("profiles.status_expiring"), tone: "warning" });
  }
  return out;
});

type Primary = { kind: "activate" | "refresh" | "edit"; label: string };
const primary = computed<Primary>(() => {
  if (!props.profile.active) return { kind: "activate", label: t("profiles.activate") };
  if (props.profile.url) return { kind: "refresh", label: t("profiles.update") };
  return { kind: "edit", label: t("profiles.raw_edit") };
});
function onPrimary() {
  if (primary.value.kind === "activate") emit("activate");
  else if (primary.value.kind === "refresh") emit("refresh");
  else emit("edit");
}
</script>

<template>
  <article class="ce-card profile-card" :class="{ 'is-active': profile.active }">
    <header class="head">
      <h3 class="name" :title="profile.name">{{ profile.name }}</h3>
      <span v-for="tag in tags" :key="tag.text" class="tag" :class="`tag-${tag.tone}`">{{ tag.text }}</span>
    </header>

    <p class="url" :title="profile.url ?? undefined">{{ profile.url ?? t("profiles.local_file") }}</p>

    <div v-if="info" class="usage">
      <div
        v-if="pct !== null"
        class="bar"
        :class="barTone"
        role="progressbar"
        :aria-valuenow="pct"
        aria-valuemin="0"
        aria-valuemax="100"
        :aria-label="t('profiles.traffic')"
      >
        <i :style="{ width: pct + '%' }"></i>
      </div>
      <div class="usage-row">
        <span class="usage-text">{{ t("profiles.traffic") }}: {{ usage }}</span>
        <span class="usage-expire" :class="{ 'is-warning': daysLeft !== null && daysLeft >= 0 && daysLeft <= EXPIRING_DAYS, 'is-danger': daysLeft !== null && daysLeft < 0 }">
          {{ expireText }}
        </span>
      </div>
    </div>

    <footer class="actions">
      <el-button
        size="small"
        :loading="primary.kind === 'refresh' && busy"
        :aria-label="`${primary.label}: ${profile.name}`"
        @click="onPrimary"
      >
        {{ primary.label }}
      </el-button>
      <el-dropdown trigger="click">
        <el-button
          size="small"
          class="card-more-btn"
          :title="t('profiles.more')"
          :aria-label="`${t('profiles.more')}: ${profile.name}`"
          aria-haspopup="menu"
        >
          <MoreHorizontal :size="16" :stroke-width="1.75" aria-hidden="true" />
        </el-button>
        <template #dropdown>
          <el-dropdown-menu>
            <el-dropdown-item v-if="profile.url && primary.kind !== 'refresh'" @click="emit('refresh')">
              {{ t("profiles.update") }}
            </el-dropdown-item>
            <el-dropdown-item @click="emit('rename')">{{ t("profiles.rename") }}</el-dropdown-item>
            <el-dropdown-item v-if="primary.kind !== 'edit'" @click="emit('edit')">{{ t("profiles.raw_edit") }}</el-dropdown-item>
            <el-dropdown-item v-if="profile.url" @click="emit('ua')">{{ t("profiles.subscribe_ua") }}</el-dropdown-item>
            <el-dropdown-item @click="emit('export')">{{ t("profiles.export") }}</el-dropdown-item>
            <el-dropdown-item divided class="danger-item" @click="emit('delete')">
              {{ t("profiles.delete") }}
            </el-dropdown-item>
          </el-dropdown-menu>
        </template>
      </el-dropdown>
    </footer>
  </article>
</template>

<style scoped>
.profile-card {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-3);
  min-width: 0;
}

.head {
  display: flex;
  align-items: center;
  gap: var(--ce-space-2);
  min-width: 0;
}

.name {
  flex: 0 1 auto;
  min-width: 0;
  margin: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font: var(--ce-type-title-3);
  color: var(--ce-text-primary);
}

.tag {
  flex: none;
  display: inline-flex;
  align-items: center;
  height: 22px;
  padding: 0 var(--ce-space-2);
  border-radius: var(--ce-radius-sm);
  font: var(--ce-type-caption);
  white-space: nowrap;
}

.tag-accent {
  background: var(--ce-accent-soft);
  color: var(--ce-accent-fg);
}

.tag-warning {
  background: var(--ce-warning-soft);
  color: var(--ce-warning-fg);
}

.tag-danger {
  background: var(--ce-danger-soft);
  color: var(--ce-danger-fg);
}

.url {
  margin: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font: var(--ce-type-mono);
  color: var(--ce-text-tertiary);
}

.usage {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-2);
}

.bar {
  height: 6px;
  overflow: hidden;
  border-radius: var(--ce-radius-full);
  background: var(--ce-fill-soft);
}

.bar > i {
  display: block;
  height: 100%;
  border-radius: var(--ce-radius-full);
  background: var(--ce-accent-bg);
}

.bar.is-warning > i {
  background: var(--ce-warning-solid);
}

.bar.is-danger > i {
  background: var(--ce-danger-solid);
}

.usage-row {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: var(--ce-space-2);
  font: var(--ce-type-callout);
  font-variant-numeric: tabular-nums;
}

.usage-text {
  color: var(--ce-text-primary);
}

.usage-expire {
  color: var(--ce-text-tertiary);
}

.usage-expire.is-warning {
  color: var(--ce-warning-fg);
}

.usage-expire.is-danger {
  color: var(--ce-danger-fg);
}

.actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--ce-space-1);
  margin-top: auto;
  padding-top: var(--ce-space-3);
  border-top: 1px solid var(--ce-divider);
}

.actions .el-button + .el-button {
  margin-left: 0;
}

.card-more-btn {
  padding: 0 var(--ce-space-2);
}
</style>

<style>
/* 下拉菜单被 teleport 到 body，scoped 选择器够不到：删除项语义红但不使用实心底。 */
.el-dropdown-menu__item.danger-item {
  color: var(--ce-danger-fg);
}
</style>
