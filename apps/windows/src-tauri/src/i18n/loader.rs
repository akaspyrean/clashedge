// src-tauri/src/i18n/loader.rs
//! 国际化加载器：YAML 点分键查找（无 fluent 依赖）
//! 单一来源：resources/i18n/*.yaml → 编译时嵌入 → 运行时扁平化为点分键表
//! 后端（托盘菜单/通知）与前端（vue-i18n）共用同一套 YAML

use once_cell::sync::Lazy;
use std::collections::HashMap;

/// 支持的语言列表
const SUPPORTED_LOCALES: &[&str] = &["zh-CN", "en-US"];
const DEFAULT_LOCALE: &str = "zh-CN";

/// 编译时嵌入并扁平化的翻译表：locale -> { "tray.control_panel" -> "控制面板" }
static RESOURCES: Lazy<HashMap<&'static str, HashMap<String, String>>> = Lazy::new(|| {
    let mut map: HashMap<&'static str, HashMap<String, String>> = HashMap::new();
    map.insert(
        "zh-CN",
        flatten_yaml(include_str!("../../resources/i18n/zh-CN.yaml")),
    );
    map.insert(
        "en-US",
        flatten_yaml(include_str!("../../resources/i18n/en-US.yaml")),
    );
    map
});

/// 将嵌套 YAML 扁平化为 "a.b.c" 键到字符串值的映射
fn flatten_yaml(yaml: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    match serde_norway::from_str::<serde_norway::Value>(yaml) {
        Ok(value) => flatten_value(&value, "", &mut out),
        // 解析失败（如同层重复键）会导致整张消息表为空、界面显示原始键名；
        // 打日志便于定位，并由下方测试在 CI 阶段拦截。
        Err(err) => eprintln!("[i18n] failed to parse embedded yaml: {err}"),
    }
    out
}

fn flatten_value(value: &serde_norway::Value, prefix: &str, out: &mut HashMap<String, String>) {
    match value {
        serde_norway::Value::Mapping(map) => {
            for (k, v) in map {
                let key = k.as_str().unwrap_or_default();
                let joined = if prefix.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", prefix, key)
                };
                flatten_value(v, &joined, out);
            }
        }
        serde_norway::Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        // 数字/布尔等其他标量：转字符串
        serde_norway::Value::Number(n) => {
            out.insert(prefix.to_string(), n.to_string());
        }
        serde_norway::Value::Bool(b) => {
            out.insert(prefix.to_string(), b.to_string());
        }
        _ => {}
    }
}

/// 简单的占位符替换：`{version}` -> 参数值
fn interpolate(template: &str, args: Option<&HashMap<String, String>>) -> String {
    let Some(args) = args else {
        return template.to_string();
    };
    let mut result = template.to_string();
    for (k, v) in args {
        result = result.replace(&format!("{{{}}}", k), v);
    }
    result
}

/// 后端翻译对象（进程内通常持有一个，随语言切换重建）
pub struct I18n {
    locale: String,
}

impl I18n {
    /// 创建指定语言的翻译对象；不支持的语言回退到默认语言
    pub fn new(locale: &str) -> Self {
        let locale = if SUPPORTED_LOCALES.contains(&locale) {
            locale
        } else {
            DEFAULT_LOCALE
        };
        Self {
            locale: locale.to_string(),
        }
    }

    /// 翻译点分键，如 "tray.control_panel"
    pub fn t(&self, key: &str) -> String {
        format_message(&self.locale, key, None)
    }
}

/// 获取指定语言翻译表中的一条消息
pub fn format_message(locale: &str, key: &str, args: Option<&HashMap<String, String>>) -> String {
    let table = RESOURCES
        .get(locale)
        .or_else(|| RESOURCES.get(DEFAULT_LOCALE));

    match table.and_then(|t| t.get(key)) {
        Some(v) => interpolate(v, args),
        None => key.to_string(),
    }
}

/// 获取指定语言的扁平化消息表（`"a.b.c" -> 消息`），供前端 vue-i18n 使用。
/// 集中配置：前端与托盘菜单共用同一份 YAML 的同一份扁平化结果。
pub fn messages_for_locale(locale: &str) -> HashMap<String, String> {
    RESOURCES
        .get(locale)
        .or_else(|| RESOURCES.get(DEFAULT_LOCALE))
        .cloned()
        .unwrap_or_default()
}

/// 获取支持的语言列表
pub fn supported_locales() -> &'static [&'static str] {
    SUPPORTED_LOCALES
}

/// 获取默认语言
pub fn default_locale() -> &'static str {
    DEFAULT_LOCALE
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 内嵌 YAML 必须能解析且非空：同层重复键会让 serde_norway 报错，
    /// 进而整张消息表为空、界面全部显示原始键名。
    #[test]
    fn embedded_yaml_parses_and_non_empty() {
        for (locale, yaml) in [
            ("zh-CN", include_str!("../../resources/i18n/zh-CN.yaml")),
            ("en-US", include_str!("../../resources/i18n/en-US.yaml")),
        ] {
            serde_norway::from_str::<serde_norway::Value>(yaml)
                .unwrap_or_else(|e| panic!("{locale} yaml invalid: {e}"));
            let table = flatten_yaml(yaml);
            assert!(
                table.len() > 100,
                "{locale} message table suspiciously small: {}",
                table.len()
            );
            // 抽查若干本轮改动过的键
            for key in [
                "general.fpm_off",
                "general.gdm_manual",
                "general.gdm_use-external",
                "general.auto_update_subscription_hint",
                "tun.auto_route_hint",
            ] {
                assert!(table.contains_key(key), "{locale} missing key {key}");
            }
        }
    }
}
