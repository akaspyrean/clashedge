<!-- 配置文件卡片：名称 / 徽标 / 订阅流量，右侧动作（激活 / 更新 / 更多菜单）。
     只发事件，不碰 store——业务由 ProfilesView 编排。 -->
<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { MoreFilled } from "@element-plus/icons-vue";
import type { ProfileInfo } from "@/api/profiles";
import { formatBytes, parseUserinfo } from "@/utils/format";

const props = defineProps<{ profile: ProfileInfo; busy?: boolean }>();
defineEmits<{
  (e: "activate"): void;
  (e: "refresh"): void;
  (e: "rename"): void;
  (e: "edit"): void;
  (e: "ua"): void;
  (e: "delete"): void;
}>();

const { t } = useI18n();

const traffic = computed(() => {
  const info = parseUserinfo(props.profile.userinfo);
  if (!info) return "";
  const total = info.total ? formatBytes(info.total) : t("profiles.traffic_unlimited");
  const out = [`${t("profiles.traffic")}: ${formatBytes(info.used)} / ${total}`];
  if (info.expire) {
    out.push(`${t("profiles.traffic_expire")}: ${new Date(info.expire * 1000).toLocaleDateString()}`);
  } else if (info.expire === 0) {
    out.push(t("profiles.traffic_never"));
  }
  return out.join(" · ");
});
</script>

<template>
  <el-card class="profile-card" :class="{ 'is-active': profile.active }">
    <div class="profile-row">
      <div class="profile-main">
        <span class="profile-name" :title="profile.name">{{ profile.name }}</span>
        <el-tag v-if="profile.active" type="success" size="small" effect="plain">
          {{ t("profiles.active") }}
        </el-tag>
        <el-tag v-if="profile.url" type="info" size="small" effect="plain" class="profile-source">
          {{ t("profiles.subscribe") }}
        </el-tag>
        <span v-if="traffic" class="profile-traffic">{{ traffic }}</span>
      </div>
      <div class="card-actions">
        <el-button
          v-if="!profile.active"
          size="small"
          type="primary"
          plain
          :aria-label="`${t('profiles.activate')}: ${profile.name}`"
          @click="$emit('activate')"
        >
          {{ t("profiles.activate") }}
        </el-button>
        <el-button
          v-if="profile.url"
          size="small"
          :loading="busy"
          :aria-label="`${t('profiles.update')}: ${profile.name}`"
          @click="$emit('refresh')"
        >
          {{ t("profiles.update") }}
        </el-button>
        <el-dropdown trigger="click">
          <el-button
            size="small"
            class="card-more-btn"
            :title="t('profiles.more')"
            :aria-label="`${t('profiles.more')}: ${profile.name}`"
            aria-haspopup="menu"
          >
            <el-icon><MoreFilled /></el-icon>
          </el-button>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item @click="$emit('rename')">{{ t("profiles.rename") }}</el-dropdown-item>
              <el-dropdown-item @click="$emit('edit')">{{ t("profiles.raw_edit") }}</el-dropdown-item>
              <el-dropdown-item v-if="profile.url" @click="$emit('ua')">
                {{ t("profiles.subscribe_ua") }}
              </el-dropdown-item>
              <el-dropdown-item divided class="danger-item" @click="$emit('delete')">
                {{ t("profiles.delete") }}
              </el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </div>
    </div>
  </el-card>
</template>

<style scoped>
.profile-card {
  --el-card-bg-color: var(--bg-raised);
  --el-card-border-color: var(--card-border);
  --el-card-padding: var(--space-3) var(--space-4);
  --el-card-border-radius: var(--r-md);
  border: 1px solid var(--card-border);
  transition: border-color 0.18s ease;
}

/* 靠底色与描边分层即可，无阴影（设计系统：卡片默认无阴影）。 */
.profile-card:hover {
  border-color: var(--border-subtle);
}

.profile-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.profile-main {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.profile-name {
  font-weight: 500;
  font-size: 14px;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.profile-source {
  flex: none;
}

.profile-traffic {
  font-size: 12px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.card-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-left: auto;
}

.card-actions .el-button + .el-button {
  margin-left: 0;
}

.card-actions .el-button {
  border-radius: var(--r-sm);
}

.card-more-btn {
  padding: 5px 8px;
}
</style>

<style>
/* 下拉菜单被 teleport 到 body，scoped 选择器够不到：删除项语义红但不使用实心底（Quiet Power）。 */
.el-dropdown-menu__item.danger-item {
  color: var(--error);
}
</style>
