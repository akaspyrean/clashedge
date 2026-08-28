// src/api/profiles.ts
// 配置文件命令的类型化封装。
// 注意：Rust 参数虽为 snake_case，但 Tauri 2 的 invoke 按 camelCase 匹配前端 key
// （oldName / newName）——这里必须传驼峰 key，否则报 missing required key。

import { invoke } from "@tauri-apps/api/core";

export interface ProfileInfo {
  name: string;
  path: string;
  active: boolean;
  /** 订阅地址（# subscribe-url: 注释头）；无则为 null，界面据此显示「更新」按钮 */
  url?: string | null;
}

export const profilesApi = {
  list: () => invoke<ProfileInfo[]>("list_profiles"),
  create: (name: string, content?: string) =>
    invoke<void>("create_profile", { name, content }),
  remove: (name: string) => invoke<void>("delete_profile", { name }),
  rename: (oldName: string, newName: string) =>
    invoke<void>("rename_profile", { oldName, newName }),
  activate: (name: string) => invoke<void>("activate_profile", { name }),
  getContent: (name: string) => invoke<string>("get_profile_content", { name }),
  updateContent: (name: string, content: string) =>
    invoke<void>("update_profile_content", { name, content }),
  import: (name: string, content: string) =>
    invoke<void>("import_profile", { name, content }),
  importFromUrl: (name: string, url: string) =>
    invoke<void>("import_profile_from_url", { name, url }),
  updateProfile: (name: string) =>
    invoke<void>("update_profile_subscription", { name }),
  export: (name: string) => invoke<string>("export_profile", { name }),
};
