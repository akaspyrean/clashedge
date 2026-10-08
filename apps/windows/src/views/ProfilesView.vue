<!-- src/views/ProfilesView.vue - 配置文件管理（独立导航页）：
     工具栏（新建 / 订阅 / [更多▾: 导入/导出]）+ 配置文件卡片列表。
     本文件只做编排：哪个对话框打开、对哪个配置操作；表单与提交逻辑在 components/profiles/*。 -->
<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowDown } from "@element-plus/icons-vue";
import ContentDialog from "@/components/profiles/ContentDialog.vue";
import ExportDialog from "@/components/profiles/ExportDialog.vue";
import ProfileCard from "@/components/profiles/ProfileCard.vue";
import RawEditDialog from "@/components/profiles/RawEditDialog.vue";
import RenameDialog from "@/components/profiles/RenameDialog.vue";
import SubscribeDialog from "@/components/profiles/SubscribeDialog.vue";
import UserAgentDialog from "@/components/profiles/UserAgentDialog.vue";
import { useConfirm } from "@/composables/useConfirm";
import { useNotify } from "@/composables/useNotify";
import { useProfilesStore } from "@/stores/profiles";

const { t } = useI18n();
const profiles = useProfilesStore();
const notify = useNotify();
const confirm = useConfirm();

onMounted(() => {
  void profiles.list();
});

// ---- 对话框可见性 + 当前操作对象 ----
const subscribeOpen = ref(false);
const newOpen = ref(false);
const importOpen = ref(false);
const exportOpen = ref(false);
const renameOpen = ref(false);
const editOpen = ref(false);
const uaOpen = ref(false);
const target = ref<string | null>(null);
const targetUa = ref<string | null>(null);

function open(kind: "rename" | "edit" | "ua", name: string, ua?: string | null) {
  target.value = name;
  targetUa.value = ua ?? null;
  if (kind === "rename") renameOpen.value = true;
  else if (kind === "edit") editOpen.value = true;
  else uaOpen.value = true;
}

// 订阅更新：按配置名粒度的在途集合，连点同一张卡不重复拉取。
const refreshing = ref(new Set<string>());

async function onActivate(name: string) {
  try {
    await profiles.activate(name);
    notify.ok();
  } catch (e) {
    notify.fail(e);
  }
}

async function onRefresh(name: string) {
  if (refreshing.value.has(name)) return;
  refreshing.value.add(name);
  try {
    await profiles.refreshSubscription(name);
    notify.ok();
  } catch (e) {
    notify.fail(e);
  } finally {
    refreshing.value.delete(name);
  }
}

async function onDelete(name: string) {
  if (
    !(await confirm(t("common.confirm"), {
      title: t("common.delete"),
      confirmText: t("common.confirm"),
      cancelText: t("profiles.cancel"),
    }))
  )
    return;
  try {
    await profiles.remove(name);
    notify.ok();
  } catch (e) {
    notify.fail(e);
  }
}
</script>

<template>
  <div class="page">
    <h2 class="page-title">{{ $t("profiles.title") }}</h2>

    <!-- 工具栏：新建（主动作）/ 订阅管理 / 更多（导入/导出收进菜单） -->
    <div class="toolbar">
      <el-button type="primary" @click="newOpen = true">{{ $t("profiles.new") }}</el-button>
      <el-button @click="subscribeOpen = true">{{ $t("profiles.subscribe_manage") }}</el-button>
      <el-dropdown trigger="click">
        <el-button aria-haspopup="menu">
          {{ $t("profiles.more") }}
          <el-icon class="toolbar-caret"><ArrowDown /></el-icon>
        </el-button>
        <template #dropdown>
          <el-dropdown-menu>
            <el-dropdown-item @click="importOpen = true">{{ $t("profiles.import") }}</el-dropdown-item>
            <el-dropdown-item @click="exportOpen = true">{{ $t("profiles.export") }}</el-dropdown-item>
          </el-dropdown-menu>
        </template>
      </el-dropdown>
    </div>

    <el-empty
      v-if="profiles.profiles.length === 0"
      :description="profiles.loading ? $t('common.loading') : $t('profiles.empty')"
    />

    <ul v-else class="profile-list" :aria-label="$t('profiles.title')">
      <li v-for="p in profiles.profiles" :key="p.name">
        <ProfileCard
          :profile="p"
          :busy="refreshing.has(p.name)"
          @activate="onActivate(p.name)"
          @refresh="onRefresh(p.name)"
          @rename="open('rename', p.name)"
          @edit="open('edit', p.name)"
          @ua="open('ua', p.name, p.user_agent)"
          @delete="onDelete(p.name)"
        />
      </li>
    </ul>

    <SubscribeDialog v-model="subscribeOpen" />
    <ContentDialog v-model="newOpen" mode="create" />
    <ContentDialog v-model="importOpen" mode="import" />
    <ExportDialog v-model="exportOpen" />
    <RenameDialog v-model="renameOpen" :target="target" />
    <RawEditDialog v-model="editOpen" :target="target" />
    <UserAgentDialog v-model="uaOpen" :target="target" :current="targetUa" />
  </div>
</template>

<style scoped>
.toolbar {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 16px;
}

.toolbar-caret {
  margin-left: 4px;
}

.profile-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin: 0;
  padding: 0;
  list-style: none;
}
</style>
