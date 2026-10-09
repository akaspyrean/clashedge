<!-- src/views/ProfilesView.vue - 订阅管理（独立导航页）：
     工具栏（新建配置 / 导入·导出▾ / 添加订阅[唯一 primary]）+ 订阅卡片网格。
     本文件只做编排：哪个对话框打开、对哪个配置操作；表单与提交逻辑在 components/profiles/*。 -->
<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { ChevronDown, Plus } from "lucide-vue-next";
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
const exportTarget = ref<string | null>(null);
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

// 订阅更新：按配置名粒度的在途集合，连点同一张卡不重复拉取；
// 失败集合仅在本次会话内记忆，用于卡片上的「更新失败」状态标签。
const refreshing = ref(new Set<string>());
const failed = ref(new Set<string>());

function onExport(name: string | null) {
  exportTarget.value = name;
  exportOpen.value = true;
}

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
    failed.value.delete(name);
    notify.ok();
  } catch (e) {
    failed.value.add(name);
    notify.fail(e);
  } finally {
    refreshing.value.delete(name);
  }
}

async function onDelete(name: string) {
  if (
    !(await confirm(t("profiles.delete_body"), {
      title: t("profiles.delete_title", { name }),
      confirmText: t("profiles.delete_confirm"),
      cancelText: t("profiles.cancel"),
      danger: true,
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

    <div class="toolbar">
      <h2 class="page-title">{{ $t("profiles.title") }}</h2>
      <el-button @click="newOpen = true">{{ $t("profiles.new") }}</el-button>
      <el-dropdown trigger="click">
        <el-button aria-haspopup="menu">
          {{ $t("profiles.import_export") }}
          <ChevronDown :size="16" :stroke-width="1.75" class="toolbar-caret" aria-hidden="true" />
        </el-button>
        <template #dropdown>
          <el-dropdown-menu>
            <el-dropdown-item @click="importOpen = true">{{ $t("profiles.import") }}</el-dropdown-item>
            <el-dropdown-item @click="onExport(null)">{{ $t("profiles.export") }}</el-dropdown-item>
          </el-dropdown-menu>
        </template>
      </el-dropdown>
      <el-button type="primary" @click="subscribeOpen = true">
        <Plus :size="16" :stroke-width="1.75" aria-hidden="true" />{{ $t("profiles.add_subscription") }}
      </el-button>
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
          :failed="failed.has(p.name)"
          @activate="onActivate(p.name)"
          @refresh="onRefresh(p.name)"
          @rename="open('rename', p.name)"
          @edit="open('edit', p.name)"
          @ua="open('ua', p.name, p.user_agent)"
          @export="onExport(p.name)"
          @delete="onDelete(p.name)"
        />
      </li>
    </ul>

    <SubscribeDialog v-model="subscribeOpen" />
    <ContentDialog v-model="newOpen" mode="create" />
    <ContentDialog v-model="importOpen" mode="import" />
    <ExportDialog v-model="exportOpen" :initial="exportTarget" />
    <RenameDialog v-model="renameOpen" :target="target" />
    <RawEditDialog v-model="editOpen" :target="target" />
    <UserAgentDialog v-model="uaOpen" :target="target" :current="targetUa" />
  </div>
</template>

<style scoped>
.toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ce-space-3);
  margin-bottom: var(--ce-space-5);
}

.toolbar .page-title {
  margin: 0 auto 0 0;
}

.toolbar-caret {
  margin-left: var(--ce-space-1);
}

.profile-list {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: var(--ce-space-4);
  margin: 0;
  padding: 0;
  list-style: none;
}

.profile-list > li {
  display: flex;
  min-width: 0;
}

.profile-list > li > :deep(*) {
  flex: 1;
}
</style>
