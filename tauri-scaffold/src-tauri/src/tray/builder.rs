// src-tauri/src/tray/builder.rs
//! System tray menu with dynamic proxy groups, modes, and speed indicator
//!
//! This module builds the system tray menu for ClashEdge.
//! It dynamically constructs the proxy group submenu based on the current
//! proxy groups configuration, and includes mode selection, connection info,
//! and speed indicator in the tray icon.

use tauri::{menu::*, tray::TrayIconBuilder, AppHandle, Manager};

use crate::config::model::Config;
use crate::core::manager::CoreStatus;
use crate::i18n::loader::I18n;

/// Proxy group info for tray menu
#[derive(Debug, Clone)]
pub struct ProxyGroupInfo {
    pub name: String,
    pub is_selected: bool,
    pub subgroups: Vec<ProxySubgroupInfo>,
}

#[derive(Debug, Clone)]
pub struct ProxySubgroupInfo {
    pub name: String,
    pub is_selected: bool,
}

/// Build the complete system tray menu
///
/// The tray menu includes:
/// - Fixed items: control panel, system proxy, TUN mode, config mixin
/// - Proxy mode selection (global/rule/direct/script)
/// - Dynamic proxy groups submenu
/// - Connections submenu
/// - More items (dev tools, restart, force quit, quit)
/// - Geo data update item
///
/// # Arguments
///
/// * `app` - Tauri app handle
/// * `_core_status` - Current core (mihomo) status (reserved for future tray status/speed display)
/// * `proxies` - List of proxy group info
/// * `config` - Application configuration
/// * `i18n` - Internationalization strings
///
/// # Returns
///
/// `Result<(Menu, TrayMenuMap)>` - 构建好的托盘菜单，以及本次构建生成的
/// 「不透明 ID → (组名, 节点名)」映射。调用方必须在 `set_menu` 成功后再
/// 用该映射调用 `replace_tray_menu_map`——先换映射后换菜单会让窗口期内
/// 的点击查到新映射、选错节点（A2 竞态）。
pub fn build_tray_menu(
    app: &AppHandle,
    _core_status: &CoreStatus,
    proxies: &[ProxyGroupInfo],
    config: &Config,
    i18n: &I18n,
) -> crate::util::error::Result<(Menu<tauri::Wry>, crate::tray::TrayMenuMap)> {
    // --- Fixed items ---

    let mut items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = vec![
        // Control panel
        Box::new(
            MenuItemBuilder::with_id("control_panel", i18n.t("tray.control_panel")).build(app)?,
        ),
        Box::new(PredefinedMenuItem::separator(app)?),
        // System proxy / TUN / config mixin check items
        // 系统代理勾选状态来自独立的 system_proxy 状态（非 allow-lan）。
        Box::new(
            CheckMenuItemBuilder::with_id("system_proxy", i18n.t("tray.system_proxy"))
                .checked(config.general.system_proxy)
                .build(app)?,
        ),
        Box::new(
            CheckMenuItemBuilder::with_id("tun_mode", i18n.t("tray.tun_mode"))
                .checked(config.tun.enable)
                .build(app)?,
        ),
        Box::new(
            CheckMenuItemBuilder::with_id("config_mixin", i18n.t("tray.config_mixin"))
                .checked(config.mixin_enabled)
                .build(app)?,
        ),
        Box::new(
            CheckMenuItemBuilder::with_id("autostart", i18n.t("tray.autostart"))
                .checked(crate::util::autostart::get_autostart().unwrap_or(false))
                .build(app)?,
        ),
        Box::new(PredefinedMenuItem::separator(app)?),
    ];

    // Proxy mode submenu
    // 注意：mihomo 仅支持 rule/global/direct；script 是 Clash Premium 遗留，
    // 菜单中不提供（否则会出现一个永远无法激活的假选项）。
    let mode_items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = vec![
        Box::new(
            CheckMenuItemBuilder::with_id("mode_global", i18n.t("tray.mode_global"))
                .checked(config.general.proxy_mode == "global")
                .build(app)?,
        ),
        Box::new(
            CheckMenuItemBuilder::with_id("mode_rule", i18n.t("tray.mode_rule"))
                .checked(config.general.proxy_mode == "rule")
                .build(app)?,
        ),
        Box::new(
            CheckMenuItemBuilder::with_id("mode_direct", i18n.t("tray.mode_direct"))
                .checked(config.general.proxy_mode == "direct")
                .build(app)?,
        ),
    ];
    let mode_refs: Vec<&dyn IsMenuItem<tauri::Wry>> =
        mode_items.iter().map(|b| b.as_ref()).collect();
    items.push(Box::new(
        SubmenuBuilder::with_id(app, "proxy_mode", i18n.t("tray.proxy_mode"))
            .items(&mode_refs)
            .build()?,
    ));

    items.push(Box::new(PredefinedMenuItem::separator(app)?));

    // Proxy groups (dynamic)
    // 每次重建菜单时生成新的 ID → 名称 映射；映射替换由调用方在 set_menu
    // 成功之后执行（保证窗口期内的点击落在「旧菜单 + 旧映射」的一致组合上）。
    let (group_items, id_map) = build_proxy_group_items(app, proxies)?;
    let group_refs: Vec<&dyn IsMenuItem<tauri::Wry>> =
        group_items.iter().map(|b| b.as_ref()).collect();
    items.push(Box::new(
        SubmenuBuilder::with_id(app, "proxy_groups", i18n.t("tray.proxy_groups"))
            .items(&group_refs)
            .build()?,
    ));

    items.push(Box::new(PredefinedMenuItem::separator(app)?));

    // Connections submenu
    let conn_items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = vec![Box::new(
        MenuItemBuilder::with_id("close_all", i18n.t("tray.close_all")).build(app)?,
    )];
    let conn_refs: Vec<&dyn IsMenuItem<tauri::Wry>> =
        conn_items.iter().map(|b| b.as_ref()).collect();
    items.push(Box::new(
        SubmenuBuilder::with_id(app, "connections", i18n.t("tray.connections"))
            .items(&conn_refs)
            .build()?,
    ));

    items.push(Box::new(PredefinedMenuItem::separator(app)?));

    // More submenu
    // dev_tools（打开 devtools）仅 debug 构建展示；release 不暴露调试面。
    // move_to_monitor 无真实实现（违反「已展示=必须可用」），菜单中移除；
    // 待 Portable Updater 阶段实现后再恢复。
    // dev_tools 是 cfg 条件条目，无法收进 vec![] 字面量：先建非空 Vec，
    // debug 构建再把 dev_tools 插到队首，菜单顺序与旧实现一致。
    #[allow(unused_mut)]
    let mut more_items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = vec![
        Box::new(MenuItemBuilder::with_id("restart", i18n.t("tray.restart")).build(app)?),
        Box::new(MenuItemBuilder::with_id("force_quit", i18n.t("tray.force_quit")).build(app)?),
    ];
    #[cfg(debug_assertions)]
    more_items.insert(
        0,
        Box::new(MenuItemBuilder::with_id("dev_tools", i18n.t("tray.dev_tools")).build(app)?),
    );
    let more_refs: Vec<&dyn IsMenuItem<tauri::Wry>> =
        more_items.iter().map(|b| b.as_ref()).collect();
    items.push(Box::new(
        SubmenuBuilder::with_id(app, "more", i18n.t("tray.more"))
            .items(&more_refs)
            .build()?,
    ));

    items.push(Box::new(PredefinedMenuItem::separator(app)?));

    // Geo data update
    items.push(Box::new(
        MenuItemBuilder::with_id("geodata_update", i18n.t("tray.geodata_update")).build(app)?,
    ));

    items.push(Box::new(PredefinedMenuItem::separator(app)?));

    // Quit
    items.push(Box::new(
        MenuItemBuilder::with_id("quit", i18n.t("tray.quit")).build(app)?,
    ));

    let item_refs: Vec<&dyn IsMenuItem<tauri::Wry>> = items.iter().map(|b| b.as_ref()).collect();
    let menu = MenuBuilder::new(app).items(&item_refs).build()?;
    Ok((menu, id_map))
}

/// Build the proxy groups submenu items dynamically
///
/// This creates the items to be placed inside the "proxy_groups" submenu.
/// A group without subgroups becomes a single checkable item; a group with
/// subgroups becomes a nested submenu whose children are one checkable item
/// per subgroup.
///
/// 审计 P1-9：MenuId 不再编码真实组名/节点名（含 `_` 时反解歧义、中文进 ID），
/// 改用按构建顺序分配的稳定序号 ID（`proxy-item-0001`），真实名称写入返回的
/// 映射（ID → (组名, 节点名)；节点名为空串表示组本身，无可选节点）。
///
/// 代理组子菜单条目 + 不透明序号 ID → (组名, 节点名) 映射。
type GroupItemsAndMap = (
    Vec<Box<dyn IsMenuItem<tauri::Wry>>>,
    crate::tray::TrayMenuMap,
);

fn build_proxy_group_items(
    app: &AppHandle,
    proxies: &[ProxyGroupInfo],
) -> crate::util::error::Result<GroupItemsAndMap> {
    let mut items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = Vec::new();
    let mut id_map: crate::tray::TrayMenuMap = std::collections::HashMap::new();
    let mut next_id: usize = 0;

    // 分配下一个不透明序号 ID，并登记其真实名称。
    macro_rules! alloc_id {
        ($entry:expr) => {{
            let id = format!("proxy-item-{:04}", next_id);
            next_id += 1;
            id_map.insert(id.clone(), $entry);
            id
        }};
    }

    for group in proxies {
        let group_name = &group.name;

        if group.subgroups.is_empty() {
            // Simple proxy group - a single checkable item（无子节点，点击不触发 select）
            let id = alloc_id!((group_name.clone(), String::new()));
            let item = CheckMenuItemBuilder::with_id(id, group_name)
                .checked(group.is_selected)
                .build(app)?;
            items.push(Box::new(item));
        } else {
            // Complex group with subgroups - a nested submenu
            let mut subgroup_items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = Vec::new();
            for subgroup in &group.subgroups {
                let id = alloc_id!((group_name.clone(), subgroup.name.clone()));
                let sub_item = CheckMenuItemBuilder::with_id(id, &subgroup.name)
                    .checked(subgroup.is_selected)
                    .build(app)?;
                subgroup_items.push(Box::new(sub_item));
            }

            let subgroup_refs: Vec<&dyn IsMenuItem<tauri::Wry>> =
                subgroup_items.iter().map(|b| b.as_ref()).collect();
            // 子菜单自身 ID 仅作容器标识，不参与事件分发，同样使用不透明序号。
            let submenu_id = format!("proxy-group-sub-{:04}", next_id);
            next_id += 1;
            let submenu = SubmenuBuilder::with_id(app, submenu_id, group_name)
                .items(&subgroup_refs)
                .build()?;
            items.push(Box::new(submenu));
        }
    }

    Ok((items, id_map))
}

/// 托盘图标：系统代理开 → 绿（活跃态）；系统代理关 → 蓝（闲置态）。
/// 运行时解码内置 32x32 PNG，按 RGBA 重绘成 `tauri::image::Image`，
/// 不额外引入图标资源文件。托盘尺寸小，32x32 源图由 Windows 自行缩放。
///
/// WCAG 1.4.3/1.4.11 对比度：单一纯色不可能同时对纯黑和纯白任务栏都 ≥4.5:1
/// （对白 ≥4.5 需 L≤0.175，对黑 ≥4.5 需 L≥0.175，二者互斥），所以按系统主题
/// 取两套配色——浅色任务栏用深色猫身+白眼，深色任务栏用浅色猫身+深眼：
/// - 浅色任务栏（AppsUseLightTheme=1）：
///   开=深绿 #0A5A2A (8.37:1)  关=深蓝 #174FA0 (7.89:1)  眼睛=白 #FFFFFF
/// - 深色任务栏（AppsUseLightTheme=0）：
///   开=浅绿 #96DFB0 (13.48:1) 关=浅蓝 #A8C8F5 (12.25:1) 眼睛=深 #242428
///
/// 眼睛-猫身对比度（小尺寸细节，按 AAA 文本级 ≥7:1 复核）：
/// 浅色开 8.37:1 / 浅色关 7.89:1 / 深色开 9.93:1 / 深色关 9.02:1 —— 均 ≥7:1。
///
/// 猫身-任务栏背景对比度（整图）：浅色开 8.37 / 浅色关 7.89 / 深色开 12.18 /
/// 深色关 11.07 —— 均 ≥7:1。眼睛永远被猫身包围（眼睛区域只在猫身内部，
/// 不触透明背景），故「眼睛-任务栏」不作为衡量项。
///
/// 源图（32x32.png）是参考包原图标：深蓝猫身 + 纯黑轮廓/耳朵 + 两个白色
/// 眼睛点。眼睛点已在源图上扩大为约 4x4（用户要求「把白点扩大一点点」），
/// 运行时不再做膨胀，直接按 sum≥380 识别白点着色，保持参考包原貌。
///
/// 历史缺陷修正：
/// - 旧实现 v1：阈值 `sum<384` 把深蓝猫身（sum≈159）也误判为眼睛，737/783 像素
///   被涂成眼色（浅色任务栏=白），整只猫几乎全白。
/// - 旧实现 v2：阈值改为 `sum<60`，但源图的眼睛是亮白像素（sum≈765）而非纯黑，
///   纯黑的 11 个像素是耳朵/轮廓——结果把耳朵当眼睛涂了眼色，真正的白眼睛
///   反而被涂成猫身色，眼睛消失。
/// - 旧实现 v3：阈值 `sum>=380` 匹配亮白眼睛，但再叠加 3+1 轮膨胀把 4x4 白点
///   扩成 ~9px 斑块（用户反馈「像真眼珠，吓人」）→ 移除膨胀，保留原图白点。
/// - 当前：阈值 `sum>=380` 精准匹配白点眼睛，猫身与眼睛各自正确着色，无膨胀。
pub fn build_tray_icon(
    config: &Config,
) -> crate::util::error::Result<tauri::image::Image<'static>> {
    let bytes = include_bytes!("../../icons/32x32.png");
    let img = image::load_from_memory(bytes)
        .map_err(|e| crate::util::error::Error::Other(format!("tray icon decode failed: {}", e)))?
        .to_rgba8();
    let (w, h) = img.dimensions();

    let light_taskbar = crate::util::autostart::is_light_taskbar();
    // 目标色：开=绿，关=蓝。浅色任务栏用深色系，深色任务栏用浅色系。
    // 浅色开=深绿 #0A5A2A（eye-vs-body 8.37:1 ≥ AAA 7:1，旧值 #0F6E33 仅 6.37:1 不达标）。
    let (body_r, body_g, body_b) = match (config.general.system_proxy, light_taskbar) {
        (true, true) => (0x0A, 0x5A, 0x2A),
        (false, true) => (0x17, 0x4F, 0xA0),
        (true, false) => (0x96, 0xDF, 0xB0),
        (false, false) => (0xA8, 0xC8, 0xF5),
    };
    // 眼睛色与猫身相反：浅色任务栏=白眼，深色任务栏=深眼。
    let (eye_r, eye_g, eye_b) = if light_taskbar {
        (0xFF, 0xFF, 0xFF)
    } else {
        (0x24, 0x24, 0x28)
    };

    // 识别眼睛像素（亮白，sum>=380）并着色。源图白点已扩为 4x4，不做膨胀。
    // 猫身是深蓝（sum≈159），轮廓/耳朵是纯黑（sum≈0），阈值 380 精准匹配白点。
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b, a] = img.get_pixel(x, y).0;
            if a == 0 {
                rgba.extend_from_slice(&[r, g, b, a]);
                continue;
            }
            let sum = (r as u32) + (g as u32) + (b as u32);
            if sum >= 380 {
                rgba.extend_from_slice(&[eye_r, eye_g, eye_b, a]);
            } else {
                rgba.extend_from_slice(&[body_r, body_g, body_b, a]);
            }
        }
    }
    Ok(tauri::image::Image::new_owned(rgba, w, h))
}

/// Create the tray icon, set the initial menu, install the on_menu_event handler,
/// and store the TrayIcon into AppState.tray. Called from main.rs setup.
pub fn build_tray(app: &AppHandle) -> crate::util::error::Result<()> {
    let i18n = I18n::new(crate::i18n::loader::default_locale());
    let (menu, id_map) =
        build_tray_menu(app, &CoreStatus::default(), &[], &Config::default(), &i18n)?;

    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::tray::events::handle_tray_event(&app, &event).await {
                    tracing::error!("Tray event error: {}", e);
                }
            });
        });

    // 初始图标按真实系统代理状态着色（config_mgr 已在 setup 中 init；
    // 启动 async 恢复系统代理阶段会再刷新一次，保证最终态一致）。
    let config = {
        let state = app.state::<crate::AppState>();
        let cfg_mgr = state.config_manager.lock().unwrap();
        cfg_mgr.get_config()
    };
    builder = builder.icon(build_tray_icon(&config)?);

    let tray = builder
        .build(app)
        .map_err(crate::util::error::Error::from)?;

    let state = app.state::<crate::AppState>();
    *state.tray.lock().unwrap() = Some(tray);

    // 初始菜单已随 builder 挂载成功，此时才登记映射（A2：先 set_menu 后换映射）
    crate::tray::replace_tray_menu_map(id_map);

    Ok(())
}

/// Rebuild the menu and apply it to an existing tray icon.
pub fn update_tray_menu(
    app: &AppHandle,
    tray: &tauri::tray::TrayIcon,
    core_status: &CoreStatus,
    proxies: &[ProxyGroupInfo],
    config: &Config,
    i18n: &I18n,
) -> crate::util::error::Result<()> {
    let (menu, id_map) = build_tray_menu(app, core_status, proxies, config, i18n)?;
    // A2：先 set_menu 成功，再原子替换 ID 映射。窗口期内的点击落在
    // 「旧菜单 + 旧映射」的一致组合上；set_menu 后到换映射前极小窗口内
    // 点击新菜单项只会查不到映射（debug 日志忽略），绝不会选错节点。
    tray.set_menu(Some(menu))
        .map_err(crate::util::error::Error::from)?;
    crate::tray::replace_tray_menu_map(id_map);
    // 图标随系统代理状态着色（refresh_tray 每次调用都会刷新）
    tray.set_icon(Some(build_tray_icon(config)?))
        .map_err(crate::util::error::Error::from)
}
