// src-tauri/src/util/region.rs
//! 节点地区/国家识别（从节点名 / 备注推断所在地）
//!
//! 各家机场的节点命名习惯差异很大，本模块采用**多策略匹配**而非单一正则，
//! 以便在不修改订阅内容的前提下尽量准确地给出地区：
//!
//! 1. **Emoji 国旗**：`🇭🇰`、`🇺🇸`、`🇯🇵` …（Unicode 区域指示符，最可靠）
//! 2. **ISO 3166-1 alpha-2 前缀代码**：`HK-01`、`US_1`、`JP 2`、`SG.3`
//! 3. **中文地区名前缀**：`香港-01`、`香港 01`、`香港节点`
//! 4. **括号内的代码 / 机场三字码**：`日本(JP)`、`(HKG)`、`[NRT]`
//! 5. **英文地区关键词**：`Hong Kong`、`Singapore`、`Los Angeles` …
//! 6. **中文地区名 / 简称**：`香港`、`日本`、`港`、`日` …
//! 7. **城市 / 机场代码表**：`LAX`、`NRT`、`东京`、`曼谷` …
//!
//! 匹配优先级严格按上述顺序（越靠前越可靠）；同一策略内优先长关键词，
//! 避免 `US` 抢先于 `United States`、`IN` 抢先于 `India` 之类的误判。
//!
//! 设计约束：
//! - **纯函数、无网络、无副作用**：仅依据名称文本推断，不访问任何外部服务。
//! - **只认已知地区**：无法识别时返回 `None`，绝不猜测——避免把节点错标成
//!   别的国家而误导用户选路。
//! - **不修改节点名**：识别结果作为独立字段附加，不改写用户可见的 name。
//! - **可扩展**：新增地区只需往 `resources/region/aliases.txt` 里加一行，无需改逻辑。

use std::collections::HashMap;
use std::sync::OnceLock;

/// 单个地区的识别规则。
///
/// `aliases` 里的字符串全部按**小写**比较（英文不区分大小写）；
/// 中文别名按原样匹配。同一别名不应出现在两个地区里（构建期由测试保证）。
struct RegionRule {
    /// 展示名（中文）
    display: &'static str,
    /// 识别别名（英文小写 / 中文原样）
    aliases: &'static [&'static str],
}

// 地区识别表：唯一来源是 resources/region/aliases.txt（`展示名\t别名,...`，
// 行顺序与别名顺序即匹配优先级），由 build.rs 生成 OUT_DIR/region_data.rs
// 并在此 include!。新增地区=往 txt 里加一行，勿改生成代码。
include!(concat!(env!("OUT_DIR"), "/region_data.rs"));

/// Emoji 国旗 → ISO alpha-2 代码。
///
/// 由 Unicode 区域指示符（U+1F1E6..U+1F1FF）两两组合成国旗 emoji；
/// 这里按码点反查 ISO alpha-2 代码。
fn flag_to_code(flag: &str) -> Option<String> {
    let mut chars = flag.chars();
    let a = chars.next()?;
    let b = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    const BASE: u32 = 0x1F1E6;
    let ia = (a as u32).checked_sub(BASE)?;
    let ib = (b as u32).checked_sub(BASE)?;
    if ia > 25 || ib > 25 {
        return None;
    }
    // 区域指示符 A..Z 对应 ASCII 'A'..'Z'
    let ca = (b'A' + ia as u8) as char;
    let cb = (b'A' + ib as u8) as char;
    let mut buf = [0u8; 4];
    Some(ca.encode_utf8(&mut buf).to_string() + cb.encode_utf8(&mut buf))
}

/// ISO 3166-1 alpha-2 代码 → 展示名（覆盖国旗/前缀/括号场景的快速路径）。
fn code_to_display(code: &str) -> Option<&'static str> {
    let upper = code.to_ascii_uppercase();
    REGION_TABLE
        .iter()
        .find(|r| r.aliases.iter().any(|a| a.eq_ignore_ascii_case(&upper)))
        .map(|r| r.display)
}

/// ISO 3166-1 alpha-2 代码 → 中文展示名（GeoIP 回退路径使用）。
///
/// 未收录的代码返回 `None`，调用方可回退为原样显示代码（如 `AE`）。
pub fn display_for_code(code: &str) -> Option<&'static str> {
    code_to_display(code)
}

/// 预编译的别名 → 展示名索引（小写英文 + 中文原样）。
fn alias_index() -> &'static HashMap<&'static str, &'static str> {
    static INDEX: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut map = HashMap::new();
        for rule in REGION_TABLE {
            for alias in rule.aliases {
                map.insert(*alias, rule.display);
            }
        }
        map
    })
}

/// 节点名 → 地区展示名。无法识别时返回 `None`。
///
/// 只做名称文本匹配，不访问网络、不查询任何第三方地理库。
pub fn detect_region(node_name: &str) -> Option<&'static str> {
    let name = node_name.trim();
    if name.is_empty() {
        return None;
    }

    // 1. Emoji 国旗（最可靠）
    static EMOJI_REGEX: OnceLock<regex::Regex> = OnceLock::new();
    let emoji_re = EMOJI_REGEX
        .get_or_init(|| regex::Regex::new(r"[\u{1F1E6}-\u{1F1FF}]{2}").expect("valid flag regex"));
    if let Some(m) = emoji_re.find(name) {
        if let Some(code) = flag_to_code(m.as_str()) {
            if let Some(display) = code_to_display(&code) {
                return Some(display);
            }
        }
    }

    // 2. 开头的前缀代码：HK-01 / HK_1 / HK 2 / HK.3
    static PREFIX_REGEX: OnceLock<regex::Regex> = OnceLock::new();
    let prefix_re = PREFIX_REGEX.get_or_init(|| {
        regex::Regex::new(r"^([A-Za-z]{2})(?:[-_. ]?\d)").expect("valid prefix regex")
    });
    if let Some(caps) = prefix_re.captures(name) {
        let code = caps
            .get(1)
            .map(|m| m.as_str())
            .unwrap_or_default()
            .to_ascii_uppercase();
        if let Some(display) = code_to_display(&code) {
            return Some(display);
        }
    }

    // 3. 括号内的代码 / 机场三字码：日本(JP)、香港(HKG)、(NRT)
    static BRACKET_REGEX: OnceLock<regex::Regex> = OnceLock::new();
    let bracket_re = BRACKET_REGEX.get_or_init(|| {
        regex::Regex::new(r"[\(\[（【]\s*([A-Za-z]{2,3})\s*[\)\]）】]")
            .expect("valid bracket regex")
    });
    if let Some(caps) = bracket_re.captures(name) {
        let token = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let upper = token.to_ascii_uppercase();
        if let Some(display) = code_to_display(&upper) {
            return Some(display);
        }
        if upper.len() == 3 {
            if let Some(display) = airport_display(&upper) {
                return Some(display);
            }
        }
    }

    // 4. 关键词匹配（长关键词优先：别名表本身按「长 → 短」排列）
    let index = alias_index();
    let lower = name.to_lowercase();
    let mut best: Option<(usize, &'static str)> = None;
    for (alias, display) in index.iter() {
        if alias
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c.is_ascii_digit())
            && alias.len() <= 3
        {
            // 纯 2-3 字母的短代码只在“独立词”位置才算，避免 "Premium" 命中 "in"
            if !contains_word(&lower, alias) {
                continue;
            }
        }
        if lower.contains(alias) || name.contains(alias) {
            let len = alias.chars().count();
            if best.map(|(bl, _)| len > bl).unwrap_or(true) {
                best = Some((len, display));
            }
        }
    }
    if let Some((_, display)) = best {
        return Some(display);
    }

    // 5. 机场三字码（作为独立词出现）：LAX / NRT / HKG
    let mut airport_best: Option<(usize, &'static str)> = None;
    for (code, display) in AIRPORT_CODES.iter() {
        if contains_word(&lower, code) {
            let len = code.len();
            if airport_best.map(|(bl, _)| len > bl).unwrap_or(true) {
                airport_best = Some((len, display));
            }
        }
    }
    airport_best.map(|(_, d)| d)
}

/// 机场三字码 → 展示名。
fn airport_display(code: &str) -> Option<&'static str> {
    AIRPORT_CODES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, d)| *d)
}

/// 判断 `needle` 是否作为独立词出现在 `haystack` 中（仅 ASCII 字母数字）。
fn contains_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut start = 0usize;
    while let Some(pos) = haystack[start..].find(needle) {
        let idx = start + pos;
        let before_ok = idx == 0
            || !haystack[..idx]
                .chars()
                .next_back()
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or(false);
        let end = idx + needle.len();
        let after_ok = end >= haystack.len()
            || !haystack[end..]
                .chars()
                .next()
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or(false);
        if before_ok && after_ok {
            return true;
        }
        start = idx + 1;
        if start >= haystack.len() {
            break;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_flag_prefix_and_keywords() {
        assert_eq!(detect_region("🇭🇰 香港 01"), Some("香港"));
        assert_eq!(detect_region("🇺🇸 US-Los Angeles"), Some("美国"));
        assert_eq!(detect_region("HK-01"), Some("香港"));
        assert_eq!(detect_region("US_2"), Some("美国"));
        assert_eq!(detect_region("SG 3"), Some("新加坡"));
        assert_eq!(detect_region("香港-01"), Some("香港"));
        assert_eq!(detect_region("日本节点"), Some("日本"));
        assert_eq!(detect_region("v2ray香港"), Some("香港"));
        assert_eq!(detect_region("Hong Kong 01"), Some("香港"));
        assert_eq!(detect_region("Los Angeles-2"), Some("美国"));
        assert_eq!(detect_region("日本(JP)"), Some("日本"));
        assert_eq!(detect_region("(HKG) 中转"), Some("香港"));
        assert_eq!(detect_region("NRT 专线"), Some("日本"));
        assert_eq!(detect_region("美国 洛杉矶 03"), Some("美国"));
    }

    #[test]
    fn unknown_names_are_not_guessed() {
        assert_eq!(detect_region("Premium-01"), None);
        assert_eq!(detect_region("直连"), None);
        assert_eq!(detect_region(""), None);
        assert_eq!(detect_region("   "), None);
    }

    /// 短代码不得在单词内部误命中（如 "Premium" 含 "in"、但不能被判成印度）。
    #[test]
    fn short_codes_require_word_boundary() {
        assert_eq!(detect_region("premium-node"), None);
        assert_eq!(detect_region("IN-01"), Some("印度"));
        assert_eq!(detect_region("node IN 02"), Some("印度"));
    }

    /// 别名不得跨地区重复（否则识别结果取决于表顺序，不可预期）。
    /// 构建期（build.rs）与运行期测试双重把关。
    #[test]
    fn aliases_are_unique_across_regions() {
        let mut seen: HashMap<&str, &str> = HashMap::new();
        for rule in REGION_TABLE {
            for alias in rule.aliases {
                if let Some(prev) = seen.insert(*alias, rule.display) {
                    panic!(
                        "alias '{}' used by both {} and {}",
                        alias, prev, rule.display
                    );
                }
            }
        }
    }

    /// 机场三字码若与别名重合，必须归属同一地区——否则括号路径的结果
    /// 取决于「先查别名再查机场码」的实现顺序，会造成不可预期歧义。
    #[test]
    fn airport_codes_match_alias_regions_when_conflicting() {
        for (code, display) in AIRPORT_CODES {
            if let Some(resolved) = code_to_display(&code.to_ascii_uppercase()) {
                assert_eq!(
                    resolved, *display,
                    "airport code {code} resolves via alias to a different region"
                );
            }
        }
    }

    /// 策略 1 回归：emoji 国旗必须独立生效——名称文本里没有任何地区
    /// 关键词时，仅靠国旗也能识别（此前的等值比较使该路径静默失效，
    /// 全靠关键词兜底得到相同结果）。
    #[test]
    fn emoji_flag_matches_without_name_text() {
        assert_eq!(detect_region("🇺🇸 Premium-01"), Some("美国"));
        assert_eq!(detect_region("🇯🇵 VIP"), Some("日本"));
        assert_eq!(detect_region("🇷🇺 节点 x"), Some("俄罗斯"));
        assert_eq!(detect_region("🇬🇧 x"), Some("英国"));
    }

    /// GeoIP 回退：ISO 代码复用同一张地区表转成中文展示名。
    #[test]
    fn geoip_codes_map_to_display_names() {
        assert_eq!(display_for_code("US"), Some("美国"));
        assert_eq!(display_for_code("hk"), Some("香港"));
        assert_eq!(display_for_code("DE"), Some("德国"));
        assert_eq!(display_for_code("ZZ"), None);
    }
}
