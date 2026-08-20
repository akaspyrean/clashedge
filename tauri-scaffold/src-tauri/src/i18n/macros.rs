// src-tauri/src/i18n/macros.rs
//! 国际化宏：简化后端翻译调用

/// 基础用法：t!("tray.control_panel")
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::i18n::loader::format_message($crate::i18n::loader::default_locale(), $key, None)
    };

    // 指定语言：t!("en-US", "tray.control_panel")
    ($locale:expr, $key:expr) => {
        $crate::i18n::loader::format_message($locale, $key, None)
    };

    // 带参数：t!("app.version", &{"version" => "1.0.0".to_string()})
    ($key:expr, $args:expr) => {
        $crate::i18n::loader::format_message(
            $crate::i18n::loader::default_locale(),
            $key,
            Some($args),
        )
    };

    // 指定语言带参数
    ($locale:expr, $key:expr, $args:expr) => {
        $crate::i18n::loader::format_message($locale, $key, Some($args))
    };
}
