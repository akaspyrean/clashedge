// src-tauri/src/tray/mod.rs
//! Tray module - system tray menu and events

pub mod builder;
pub mod events;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// 托盘菜单「不透明 ID → (组名, 节点名)」映射。
///
/// 背景（审计 P1-9）：旧实现把真实组名/节点名编码进 MenuId（如
/// `proxy_group_{group}_{proxy}`），事件侧再用 `rsplitn(2, '_')` 反解；
/// 名称含 `_` 时解析歧义会选错节点，中文/空格也会进 ID。
/// 现在菜单构建时按顺序分配稳定序号 ID（`proxy-item-0001`），
/// 真实名称只存这里；refresh_tray 每次 update_tray_menu 都整体重建菜单，
/// 且映射替换严格发生在 set_menu 成功之后（保证窗口期点击语义一致）。
pub type TrayMenuMap = HashMap<String, (String, String)>;

static TRAY_MENU_MAP: OnceLock<Mutex<TrayMenuMap>> = OnceLock::new();

fn tray_menu_map() -> &'static Mutex<TrayMenuMap> {
    TRAY_MENU_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 用本次构建的映射整体替换旧映射（与菜单重建一一对应）。
pub fn replace_tray_menu_map(map: TrayMenuMap) {
    *tray_menu_map().lock().unwrap() = map;
}

/// 事件处理侧查询：菜单项 ID → (组名, 节点名)。节点名为空串表示该 ID
/// 对应组本身（无可选节点），不应触发 select。
pub fn lookup_tray_menu_item(id: &str) -> Option<(String, String)> {
    tray_menu_map().lock().unwrap().get(id).cloned()
}
