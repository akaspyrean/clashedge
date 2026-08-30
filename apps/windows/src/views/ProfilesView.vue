<!-- src/views/ProfilesView.vue - 配置文件管理（独立导航页）：
     页面级完整管理：工具栏（订阅 / 新建 / [更多▾: 导入/导出]）+ 配置文件卡片列表
     （激活 / 更新 / 重命名 / 原始编辑 / 删除）。 -->
<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { ElMessage, ElMessageBox } from "element-plus";
import { ArrowDown, MoreFilled } from "@element-plus/icons-vue";
import { profilesApi } from "@/api/profiles";
import { useProfilesStore } from "@/stores/profiles";
import { friendlyError } from "@/errors";

const { t } = useI18n();
const profilesStore = useProfilesStore();

// 对话框提交 in-flight 守卫：同一时刻只允许一个提交进行（对话框互斥打开），
// 双击/连点确认按钮时直接忽略后续触发，避免重复创建/导入/订阅。
const submitting = ref(false);

onMounted(() => {
  void profilesStore.list();
});

// ---- 订阅（URL 导入）----
const subscribeVisible = ref(false);
const subscribeUrl = ref("");
const subscribeName = ref("");

async function onSubscribe() {
  if (submitting.value) return;
  const url = subscribeUrl.value.trim();
  if (!url) return;
  submitting.value = true;
  try {
    await profilesApi.importFromUrl(subscribeName.value.trim(), url);
    await profilesStore.list();
    ElMessage.success(t("common.success"));
    subscribeVisible.value = false;
    subscribeUrl.value = "";
    subscribeName.value = "";
  } catch (e) {
    ElMessage.error(friendlyError(e));
  } finally {
    submitting.value = false;
  }
}

// ---- 新建 ----
const newVisible = ref(false);
const newName = ref("");
const newContent = ref("");

async function onCreate() {
  if (submitting.value) return;
  const name = newName.value.trim();
  if (!name) return;
  submitting.value = true;
  try {
    await profilesStore.create(name, newContent.value);
    ElMessage.success(t("common.success"));
    newVisible.value = false;
    newName.value = "";
    newContent.value = "";
  } catch (e) {
    ElMessage.error(friendlyError(e));
  } finally {
    submitting.value = false;
  }
}

// ---- 导入（[更多▾]）----
const importVisible = ref(false);
const importName = ref("");
const importContent = ref("");

async function onImport() {
  if (submitting.value) return;
  const name = importName.value.trim();
  if (!name) return;
  submitting.value = true;
  try {
    await profilesApi.import(name, importContent.value);
    await profilesStore.list();
    ElMessage.success(t("common.success"));
    importVisible.value = false;
    importName.value = "";
    importContent.value = "";
  } catch (e) {
    ElMessage.error(friendlyError(e));
  } finally {
    submitting.value = false;
  }
}

// ---- 导出（[更多▾]）----
const exportVisible = ref(false);
const exportTarget = ref("");
const exportContent = ref("");

function onExportOpen() {
  exportContent.value = "";
  exportTarget.value = profilesStore.profiles[0]?.name ?? "";
  exportVisible.value = true;
}

async function onExport() {
  if (submitting.value) return;
  const name = exportTarget.value;
  if (!name) return;
  submitting.value = true;
  try {
    exportContent.value = await profilesApi.export(name);
  } catch (e) {
    ElMessage.error(friendlyError(e));
  } finally {
    submitting.value = false;
  }
}

// ---- 激活 ----
async function onActivate(name: string) {
  try {
    await profilesStore.activate(name);
    ElMessage.success(t("common.success"));
  } catch (e) {
    ElMessage.error(friendlyError(e));
  }
}

// ---- 更新订阅（重新拉取 URL 内容；激活中则后端热重载生效）----
async function onUpdate(name: string) {
  try {
    await profilesApi.updateProfile(name);
    await profilesStore.list();
    ElMessage.success(t("common.success"));
  } catch (e) {
    ElMessage.error(friendlyError(e));
  }
}

// ---- 重命名 ----
const renameVisible = ref(false);
const renaming = ref<string | null>(null);
const renameNewName = ref("");

function onRenameOpen(name: string) {
  renaming.value = name;
  renameNewName.value = name;
  renameVisible.value = true;
}

async function onRename() {
  if (submitting.value) return;
  if (renaming.value === null) return;
  const newName = renameNewName.value.trim();
  if (!newName) return;
  submitting.value = true;
  try {
    await profilesStore.rename(renaming.value, newName);
    ElMessage.success(t("common.success"));
    renameVisible.value = false;
  } catch (e) {
    ElMessage.error(friendlyError(e));
  } finally {
    submitting.value = false;
  }
}

// ---- 编辑（原始内容）----
const editVisible = ref(false);
const editing = ref<string | null>(null);
const editContent = ref("");

function onEditOpen(name: string) {
  editing.value = name;
  editVisible.value = true;
}

async function onEditDialogOpen() {
  if (editing.value === null) return;
  editContent.value = "";
  try {
    editContent.value = await profilesApi.getContent(editing.value);
  } catch (e) {
    ElMessage.error(friendlyError(e));
  }
}

async function onEditSave() {
  if (submitting.value) return;
  if (editing.value === null) return;
  submitting.value = true;
  try {
    await profilesApi.updateContent(editing.value, editContent.value);
    ElMessage.success(t("common.success"));
    editVisible.value = false;
  } catch (e) {
    ElMessage.error(friendlyError(e));
  } finally {
    submitting.value = false;
  }
}

// ---- 删除 ----
async function onDelete(name: string) {
  try {
    await ElMessageBox.confirm(t("common.confirm"), t("common.delete"), {
      type: "warning",
      confirmButtonText: t("common.confirm"),
      cancelButtonText: t("profiles.cancel"),
    });
  } catch {
    return;
  }
  try {
    await profilesStore.remove(name);
    ElMessage.success(t("common.success"));
  } catch (e) {
    ElMessage.error(friendlyError(e));
  }
}
</script>

<template>
  <div class="page">
    <h2 class="page-title">{{ $t("profiles.title") }}</h2>

    <!-- 工具栏：新建（主动作）/ 订阅管理 / 更多（导入/导出收进菜单） -->
    <div class="toolbar">
      <el-button type="primary" @click="newVisible = true">
        {{ $t("profiles.new") }}
      </el-button>
      <el-button @click="subscribeVisible = true">
        {{ $t("profiles.subscribe_manage") }}
      </el-button>
      <el-dropdown trigger="click">
        <el-button>
          {{ $t("profiles.more") }}
          <el-icon class="toolbar-caret"><ArrowDown /></el-icon>
        </el-button>
        <template #dropdown>
          <el-dropdown-menu>
            <el-dropdown-item @click="importVisible = true">
              {{ $t("profiles.import") }}
            </el-dropdown-item>
            <el-dropdown-item @click="onExportOpen">
              {{ $t("profiles.export") }}
            </el-dropdown-item>
          </el-dropdown-menu>
        </template>
      </el-dropdown>
    </div>

    <el-empty
      v-if="profilesStore.profiles.length === 0"
      :description="profilesStore.loading
        ? $t('common.loading')
        : $t('profiles.empty')"
    />

    <div v-else class="profile-list">
      <el-card
        v-for="profile in profilesStore.profiles"
        :key="profile.name"
        class="profile-card"
      >
        <div class="profile-row">
          <div class="profile-main">
            <span class="profile-name" :title="profile.name">{{ profile.name }}</span>
            <el-tag v-if="profile.active" type="success" size="small" effect="plain">
              {{ $t("profiles.active") }}
            </el-tag>
            <el-tag v-if="profile.url" type="info" size="small" effect="plain" class="profile-source">
              {{ $t("profiles.subscribe") }}
            </el-tag>
          </div>
          <div class="card-actions">
           <el-button
            v-if="!profile.active"
            size="small"
            type="primary"
            plain
            @click="onActivate(profile.name)"
          >
            {{ $t("profiles.activate") }}
          </el-button>
          <el-button
            v-if="profile.url"
            size="small"
            @click="onUpdate(profile.name)"
          >
            {{ $t("profiles.update") }}
          </el-button>
          <el-dropdown trigger="click">
            <el-button size="small" :title="$t('profiles.more')" class="card-more-btn">
              <el-icon><MoreFilled /></el-icon>
            </el-button>
            <template #dropdown>
              <el-dropdown-menu>
                <el-dropdown-item @click="onRenameOpen(profile.name)">
                  {{ $t("profiles.rename") }}
                </el-dropdown-item>
                <el-dropdown-item @click="onEditOpen(profile.name)">
                  {{ $t("profiles.raw_edit") }}
                </el-dropdown-item>
                <el-dropdown-item divided class="danger-item" @click="onDelete(profile.name)">
                  {{ $t("profiles.delete") }}
                </el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
          </div>
        </div>
      </el-card>
    </div>

    <!-- 订阅配置 -->
    <el-dialog v-model="subscribeVisible" :title="$t('profiles.subscribe')" width="min(640px, calc(100vw - 32px))">
      <el-form label-position="top">
        <el-form-item :label="$t('profiles.subscribe_url')">
          <el-input
            v-model="subscribeUrl"
            :placeholder="$t('profiles.url_placeholder')"
          />
        </el-form-item>
        <el-form-item :label="$t('profiles.name_optional')">
          <el-input
            v-model="subscribeName"
            :placeholder="$t('profiles.name_optional_hint')"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="subscribeVisible = false">{{ $t("profiles.cancel") }}</el-button>
        <el-button type="primary" :loading="submitting" :disabled="!subscribeUrl.trim()" @click="onSubscribe">
          {{ $t("profiles.subscribe") }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 新建配置 -->
    <el-dialog v-model="newVisible" :title="$t('profiles.new')" width="min(640px, calc(100vw - 32px))">
      <el-form label-position="top">
        <el-form-item :label="$t('profiles.name')">
          <el-input v-model="newName" :placeholder="$t('profiles.name')" />
        </el-form-item>
        <el-form-item :label="$t('profiles.content')">
          <el-input v-model="newContent" type="textarea" :rows="10" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="newVisible = false">{{ $t("profiles.cancel") }}</el-button>
        <el-button type="primary" :loading="submitting" :disabled="!newName.trim()" @click="onCreate">
          {{ $t("profiles.save") }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 导入配置 -->
    <el-dialog v-model="importVisible" :title="$t('profiles.import')" width="min(640px, calc(100vw - 32px))">
      <el-form label-position="top">
        <el-form-item :label="$t('profiles.name')">
          <el-input v-model="importName" :placeholder="$t('profiles.name')" />
        </el-form-item>
        <el-form-item :label="$t('profiles.content')">
          <el-input v-model="importContent" type="textarea" :rows="10" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="importVisible = false">{{ $t("profiles.cancel") }}</el-button>
        <el-button type="primary" :loading="submitting" :disabled="!importName.trim()" @click="onImport">
          {{ $t("profiles.save") }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 导出配置 -->
    <el-dialog v-model="exportVisible" :title="$t('profiles.export')" width="min(640px, calc(100vw - 32px))">
      <el-form label-position="top">
        <el-form-item :label="$t('profiles.name')">
          <el-select v-model="exportTarget" style="width: 100%">
            <el-option
              v-for="profile in profilesStore.profiles"
              :key="profile.name"
              :label="profile.name"
              :value="profile.name"
            />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('profiles.content')">
          <el-input
            v-model="exportContent"
            type="textarea"
            :rows="10"
            readonly
            class="export-textarea"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="exportVisible = false">{{ $t("profiles.cancel") }}</el-button>
        <el-button type="primary" :loading="submitting" :disabled="!exportTarget" @click="onExport">
          {{ $t("profiles.export") }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 重命名 -->
    <el-dialog v-model="renameVisible" :title="$t('profiles.rename')" width="min(640px, calc(100vw - 32px))">
      <el-form label-position="top">
        <el-form-item :label="$t('profiles.name')">
          <el-input v-model="renameNewName" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="renameVisible = false">{{ $t("profiles.cancel") }}</el-button>
        <el-button type="primary" :loading="submitting" :disabled="!renameNewName.trim()" @click="onRename">
          {{ $t("profiles.save") }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 编辑原始内容 -->
    <el-dialog
      v-model="editVisible"
      :title="$t('profiles.raw_edit')"
      width="min(640px, calc(100vw - 32px))"
      @open="onEditDialogOpen"
    >
      <el-input
        v-model="editContent"
        type="textarea"
        :rows="16"
        class="edit-textarea"
      />
      <template #footer>
        <el-button @click="editVisible = false">{{ $t("profiles.cancel") }}</el-button>
        <el-button type="primary" :loading="submitting" @click="onEditSave">
          {{ $t("profiles.save") }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.toolbar {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 16px;
}

.profile-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

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

/* 单卡内：名称/徽标居左，操作按钮居右，紧凑单行 */
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

/* ⋯ 按钮与工具栏下拉箭头：紧凑、去多余内边距。 */
.card-more-btn {
  padding: 5px 8px;
}

.toolbar-caret {
  margin-left: 4px;
}

/* 下拉菜单中的删除项：语义红但不使用实心底（Quiet Power）。 */
.card-actions :global(.danger-item) {
  color: var(--error);
}

.edit-textarea,
.export-textarea {
  font-family: "Consolas", "Menlo", monospace;
}
</style>
