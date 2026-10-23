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
//! - **可扩展**：新增地区只需往 `REGION_TABLE` 里加一行，无需改逻辑。

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

/// 地区识别表（唯一的地区来源，便于扩展与审计）。
///
/// 顺序即匹配优先级：靠前优先命中。长别名（如 "united states"）必须排在
/// 短别名（如 "us"）之前，否则短别名会抢先误命中。
const REGION_TABLE: &[RegionRule] = &[
    RegionRule {
        display: "香港",
        aliases: &["hong kong", "hongkong", "hkg", "hk", "港", "香港"],
    },
    RegionRule {
        display: "澳门",
        aliases: &["macao", "macau", "mof", "mo", "澳门", "澳門"],
    },
    RegionRule {
        display: "台湾",
        aliases: &["taiwan", "tw", "台", "台湾", "臺灣"],
    },
    RegionRule {
        display: "日本",
        aliases: &[
            "japan",
            "tokyo",
            "osaka",
            "nagoya",
            "fukuoka",
            "sapporo",
            "nrt",
            "hnd",
            "kix",
            "fuk",
            "cts",
            "itm",
            "jp",
            "日",
            "日本",
            "东京",
            "大阪",
            "名古屋",
            "福冈",
            "札幌",
        ],
    },
    RegionRule {
        display: "韩国",
        aliases: &[
            "korea",
            "south korea",
            "seoul",
            "busan",
            "incheon",
            "icn",
            "kr",
            "韩",
            "韩国",
            "首尔",
            "釜山",
            "仁川",
        ],
    },
    RegionRule {
        display: "新加坡",
        aliases: &["singapore", "sin", "sg", "新", "新加坡"],
    },
    RegionRule {
        display: "美国",
        aliases: &[
            "united states",
            "united states of america",
            "usa",
            "los angeles",
            "san francisco",
            "new york",
            "seattle",
            "chicago",
            "dallas",
            "houston",
            "miami",
            "atlanta",
            "boston",
            "washington",
            "phoenix",
            "denver",
            "las vegas",
            "san jose",
            "san diego",
            "portland",
            "buffalo",
            "tucson",
            "philadelphia",
            "honolulu",
            "lax",
            "sfo",
            "jfk",
            "ord",
            "dfw",
            "atl",
            "mia",
            "sea",
            "iad",
            "bos",
            "den",
            "phx",
            "las",
            "mco",
            "dtw",
            "ewr",
            "clt",
            "sjd",
            "us",
            "美",
            "美国",
            "洛杉矶",
            "圣何塞",
            "旧金山",
            "纽约",
            "西雅图",
            "芝加哥",
            "达拉斯",
            "休斯顿",
            "迈阿密",
            "亚特兰大",
            "波士顿",
            "华盛顿",
            "凤凰城",
            "丹佛",
            "拉斯维加斯",
            "圣迭戈",
            "波特兰",
            "费利蒙",
        ],
    },
    RegionRule {
        display: "英国",
        aliases: &[
            "united kingdom",
            "great britain",
            "england",
            "scotland",
            "london",
            "manchester",
            "edinburgh",
            "lhr",
            "lgw",
            "stn",
            "ltn",
            "edi",
            "uk",
            "gb",
            "英",
            "英国",
            "伦敦",
            "曼彻斯特",
            "爱丁堡",
        ],
    },
    RegionRule {
        display: "德国",
        aliases: &[
            "germany",
            "frankfurt",
            "munich",
            "berlin",
            "hamburg",
            "fra",
            "muc",
            "dus",
            "ber",
            "ham",
            "de",
            "德",
            "德国",
            "法兰克福",
            "慕尼黑",
            "柏林",
            "汉堡",
        ],
    },
    RegionRule {
        display: "法国",
        aliases: &[
            "france",
            "paris",
            "lyon",
            "marseille",
            "cdg",
            "ory",
            "nce",
            "fr",
            "法",
            "法国",
            "巴黎",
            "里昂",
            "马赛",
        ],
    },
    RegionRule {
        display: "加拿大",
        aliases: &[
            "canada",
            "toronto",
            "vancouver",
            "montreal",
            "ottawa",
            "calgary",
            "yyz",
            "yvr",
            "yul",
            "ca",
            "加",
            "加拿大",
            "多伦多",
            "温哥华",
            "蒙特利尔",
            "渥太华",
            "卡尔加里",
        ],
    },
    RegionRule {
        display: "澳大利亚",
        aliases: &[
            "australia",
            "sydney",
            "melbourne",
            "brisbane",
            "perth",
            "adelaide",
            "canberra",
            "syd",
            "mel",
            "bne",
            "per",
            "adl",
            "au",
            "澳",
            "澳大利亚",
            "悉尼",
            "墨尔本",
            "布里斯班",
            "珀斯",
            "阿德莱德",
            "堪培拉",
        ],
    },
    RegionRule {
        display: "新西兰",
        aliases: &[
            "new zealand",
            "auckland",
            "wellington",
            "christchurch",
            "akl",
            "wlg",
            "nz",
            "新西兰",
            "奥克兰",
            "惠灵顿",
            "基督城",
        ],
    },
    RegionRule {
        display: "俄罗斯",
        aliases: &[
            "russia",
            "moscow",
            "saint petersburg",
            "ru",
            "俄",
            "俄罗斯",
            "莫斯科",
            "圣彼得堡",
        ],
    },
    RegionRule {
        display: "印度",
        aliases: &[
            "india",
            "mumbai",
            "delhi",
            "bangalore",
            "chennai",
            "new delhi",
            "bom",
            "del",
            "blr",
            "maa",
            "in",
            "印",
            "印度",
            "孟买",
            "新德里",
            "班加罗尔",
            "金奈",
        ],
    },
    RegionRule {
        display: "泰国",
        aliases: &[
            "thailand",
            "bangkok",
            "chiang mai",
            "phuket",
            "pattaya",
            "bkk",
            "dmk",
            "cnx",
            "hkt",
            "th",
            "泰",
            "泰国",
            "曼谷",
            "清迈",
            "普吉",
            "芭提雅",
        ],
    },
    RegionRule {
        display: "马来西亚",
        aliases: &[
            "malaysia",
            "kuala lumpur",
            "penang",
            "johor bahru",
            "kul",
            "pen",
            "jhb",
            "my",
            "马来",
            "马来西亚",
            "吉隆坡",
            "槟城",
            "新山",
        ],
    },
    RegionRule {
        display: "印度尼西亚",
        aliases: &[
            "indonesia",
            "jakarta",
            "bali",
            "cgk",
            "dps",
            "sub",
            "id",
            "印尼",
            "印度尼西亚",
            "雅加达",
            "巴厘岛",
        ],
    },
    RegionRule {
        display: "菲律宾",
        aliases: &[
            "philippines",
            "manila",
            "cebu",
            "makati",
            "mnl",
            "ceb",
            "crk",
            "ph",
            "菲律",
            "菲律宾",
            "马尼拉",
            "宿务",
            "马卡蒂",
        ],
    },
    RegionRule {
        display: "越南",
        aliases: &[
            "vietnam",
            "ho chi minh",
            "hanoi",
            "da nang",
            "sgn",
            "han",
            "dad",
            "vn",
            "越南",
            "胡志明",
            "河内",
            "岘港",
        ],
    },
    RegionRule {
        display: "巴西",
        aliases: &[
            "brazil",
            "sao paulo",
            "rio de janeiro",
            "gru",
            "gig",
            "bsb",
            "cnf",
            "br",
            "巴西",
            "圣保罗",
            "里约热内卢",
        ],
    },
    RegionRule {
        display: "阿根廷",
        aliases: &[
            "argentina",
            "buenos aires",
            "eze",
            "aep",
            "ar",
            "阿根廷",
            "布宜诺斯艾利斯",
        ],
    },
    RegionRule {
        display: "墨西哥",
        aliases: &[
            "mexico",
            "mexico city",
            "mex",
            "cun",
            "mx",
            "墨西哥",
            "墨西哥城",
        ],
    },
    RegionRule {
        display: "智利",
        aliases: &["chile", "santiago", "scl", "cl", "智利", "圣地亚哥"],
    },
    RegionRule {
        display: "南非",
        aliases: &[
            "south africa",
            "johannesburg",
            "cape town",
            "jnb",
            "cpt",
            "za",
            "南非",
            "约翰内斯堡",
            "开普敦",
        ],
    },
    RegionRule {
        display: "阿联酋",
        aliases: &[
            "united arab emirates",
            "dubai",
            "dxb",
            "ae",
            "阿联酋",
            "迪拜",
        ],
    },
    RegionRule {
        display: "沙特阿拉伯",
        aliases: &[
            "saudi arabia",
            "riyadh",
            "jeddah",
            "ruh",
            "jed",
            "sa",
            "沙特",
            "沙特阿拉伯",
            "利雅得",
            "吉达",
        ],
    },
    RegionRule {
        display: "土耳其",
        aliases: &[
            "turkey",
            "turkiye",
            "istanbul",
            "ist",
            "tr",
            "土耳其",
            "伊斯坦布尔",
        ],
    },
    RegionRule {
        display: "以色列",
        aliases: &["israel", "tel aviv", "tlv", "il", "以色列", "特拉维夫"],
    },
    RegionRule {
        display: "荷兰",
        aliases: &[
            "netherlands",
            "holland",
            "amsterdam",
            "rotterdam",
            "ams",
            "nl",
            "荷兰",
            "阿姆斯特丹",
            "鹿特丹",
        ],
    },
    RegionRule {
        display: "瑞士",
        aliases: &[
            "switzerland",
            "zurich",
            "geneva",
            "zrh",
            "gva",
            "brn",
            "ch",
            "瑞士",
            "苏黎世",
            "日内瓦",
        ],
    },
    RegionRule {
        display: "瑞典",
        aliases: &[
            "sweden",
            "stockholm",
            "gothenburg",
            "arn",
            "got",
            "sto",
            "se",
            "瑞典",
            "斯德哥尔摩",
            "哥德堡",
        ],
    },
    RegionRule {
        display: "挪威",
        aliases: &[
            "norway",
            "oslo",
            "bergen",
            "osl",
            "trd",
            "svg",
            "no",
            "挪威",
            "奥斯陆",
            "卑尔根",
        ],
    },
    RegionRule {
        display: "芬兰",
        aliases: &[
            "finland",
            "helsinki",
            "hel",
            "tmp",
            "fi",
            "芬兰",
            "赫尔辛基",
        ],
    },
    RegionRule {
        display: "丹麦",
        aliases: &[
            "denmark",
            "copenhagen",
            "cph",
            "bll",
            "dk",
            "丹麦",
            "哥本哈根",
        ],
    },
    RegionRule {
        display: "波兰",
        aliases: &[
            "poland",
            "warsaw",
            "krakow",
            "waw",
            "krk",
            "pl",
            "波兰",
            "华沙",
            "克拉科夫",
        ],
    },
    RegionRule {
        display: "捷克",
        aliases: &[
            "czech republic",
            "czechia",
            "prague",
            "prg",
            "cz",
            "捷克",
            "布拉格",
        ],
    },
    RegionRule {
        display: "奥地利",
        aliases: &["austria", "vienna", "vie", "at", "奥地利", "维也纳"],
    },
    RegionRule {
        display: "比利时",
        aliases: &[
            "belgium",
            "brussels",
            "bru",
            "crl",
            "be",
            "比利时",
            "布鲁塞尔",
        ],
    },
    RegionRule {
        display: "爱尔兰",
        aliases: &["ireland", "dublin", "dub", "ork", "ie", "爱尔兰", "都柏林"],
    },
    RegionRule {
        display: "葡萄牙",
        aliases: &["portugal", "lisbon", "lis", "opo", "pt", "葡萄牙", "里斯本"],
    },
    RegionRule {
        display: "西班牙",
        aliases: &[
            "spain",
            "madrid",
            "barcelona",
            "mad",
            "bcn",
            "es",
            "西班牙",
            "马德里",
            "巴塞罗那",
        ],
    },
    RegionRule {
        display: "意大利",
        aliases: &[
            "italy",
            "milan",
            "rome",
            "fco",
            "mxp",
            "it",
            "意大利",
            "米兰",
            "罗马",
        ],
    },
    RegionRule {
        display: "罗马尼亚",
        aliases: &[
            "romania",
            "bucharest",
            "otp",
            "ro",
            "罗马尼亚",
            "布加勒斯特",
        ],
    },
    RegionRule {
        display: "匈牙利",
        aliases: &["hungary", "budapest", "bud", "hu", "匈牙利", "布达佩斯"],
    },
    RegionRule {
        display: "保加利亚",
        aliases: &["bulgaria", "sofia", "sof", "bg", "保加利亚", "索非亚"],
    },
    RegionRule {
        display: "克罗地亚",
        aliases: &["croatia", "zagreb", "zag", "hr", "克罗地亚", "萨格勒布"],
    },
    RegionRule {
        display: "斯洛文尼亚",
        aliases: &[
            "slovenia",
            "ljubljana",
            "lju",
            "si",
            "斯洛文尼亚",
            "卢布尔雅那",
        ],
    },
    RegionRule {
        display: "斯洛伐克",
        aliases: &[
            "slovakia",
            "bratislava",
            "bts",
            "sk",
            "斯洛伐克",
            "布拉迪斯拉发",
        ],
    },
    RegionRule {
        display: "立陶宛",
        aliases: &["lithuania", "vilnius", "vno", "lt", "立陶宛", "维尔纽斯"],
    },
    RegionRule {
        display: "拉脱维亚",
        aliases: &["latvia", "riga", "rix", "lv", "拉脱维亚", "里加"],
    },
    RegionRule {
        display: "爱沙尼亚",
        aliases: &["estonia", "tallinn", "tll", "ee", "爱沙尼亚", "塔林"],
    },
    RegionRule {
        display: "马耳他",
        aliases: &["malta", "valletta", "mla", "mt", "马耳他", "瓦莱塔"],
    },
    RegionRule {
        display: "塞浦路斯",
        aliases: &[
            "cyprus",
            "nicosia",
            "lca",
            "pfo",
            "cy",
            "塞浦路斯",
            "尼科西亚",
        ],
    },
    RegionRule {
        display: "卢森堡",
        aliases: &["luxembourg", "lux", "lu", "卢森堡"],
    },
    RegionRule {
        display: "冰岛",
        aliases: &["iceland", "reykjavik", "kef", "is", "冰岛", "雷克雅未克"],
    },
    RegionRule {
        display: "列支敦士登",
        aliases: &["liechtenstein", "vaduz", "li", "列支敦士登", "瓦都兹"],
    },
    RegionRule {
        display: "摩纳哥",
        aliases: &["monaco", "mc", "摩纳哥"],
    },
    RegionRule {
        display: "圣马力诺",
        aliases: &["san marino", "sm", "圣马力诺"],
    },
    RegionRule {
        display: "梵蒂冈",
        aliases: &["vatican", "vatican city", "va", "梵蒂冈", "梵蒂冈城"],
    },
    RegionRule {
        display: "中国",
        aliases: &[
            "china",
            "mainland china",
            "beijing",
            "shanghai",
            "shenzhen",
            "guangzhou",
            "hangzhou",
            "nanjing",
            "chengdu",
            "wuhan",
            "xian",
            "chongqing",
            "tianjin",
            "suzhou",
            "xiamen",
            "qingdao",
            "changsha",
            "zhengzhou",
            "cn",
            "中",
            "中国",
            "北京",
            "上海",
            "深圳",
            "广州",
            "杭州",
            "南京",
            "成都",
            "武汉",
            "西安",
            "重庆",
            "天津",
            "苏州",
            "厦门",
            "青岛",
            "长沙",
            "郑州",
        ],
    },
];

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

/// ISO 3166-1 alpha-2 代码 → 展示名（覆盖国旗场景的快速路径）。
fn code_to_display(code: &str) -> Option<&'static str> {
    let upper = code.to_ascii_uppercase();
    REGION_TABLE
        .iter()
        .find(|r| r.aliases.iter().any(|a| *a == upper))
        .map(|r| r.display)
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

/// 机场三字码表（IATA 常见节点出口）。
const AIRPORT_CODES: &[(&str, &str)] = &[
    ("HKG", "香港"),
    ("MFM", "澳门"),
    ("TPE", "台湾"),
    ("KHH", "台湾"),
    ("NRT", "日本"),
    ("HND", "日本"),
    ("KIX", "日本"),
    ("FUK", "日本"),
    ("CTS", "日本"),
    ("OKA", "日本"),
    ("ICN", "韩国"),
    ("GMP", "韩国"),
    ("SIN", "新加坡"),
    ("BKK", "泰国"),
    ("DMK", "泰国"),
    ("CNX", "泰国"),
    ("HKT", "泰国"),
    ("KUL", "马来西亚"),
    ("PEN", "马来西亚"),
    ("JHB", "马来西亚"),
    ("CGK", "印度尼西亚"),
    ("DPS", "印度尼西亚"),
    ("SUB", "印度尼西亚"),
    ("MNL", "菲律宾"),
    ("CEB", "菲律宾"),
    ("CRK", "菲律宾"),
    ("SGN", "越南"),
    ("HAN", "越南"),
    ("DAD", "越南"),
    ("BOM", "印度"),
    ("DEL", "印度"),
    ("BLR", "印度"),
    ("SYD", "澳大利亚"),
    ("MEL", "澳大利亚"),
    ("BNE", "澳大利亚"),
    ("PER", "澳大利亚"),
    ("AKL", "新西兰"),
    ("WLG", "新西兰"),
    ("YYZ", "加拿大"),
    ("YVR", "加拿大"),
    ("YUL", "加拿大"),
    ("LAX", "美国"),
    ("SFO", "美国"),
    ("JFK", "美国"),
    ("ORD", "美国"),
    ("DFW", "美国"),
    ("ATL", "美国"),
    ("MIA", "美国"),
    ("SEA", "美国"),
    ("IAD", "美国"),
    ("BOS", "美国"),
    ("DEN", "美国"),
    ("PHX", "美国"),
    ("LAS", "美国"),
    ("EWR", "美国"),
    ("LAX", "美国"),
    ("GRU", "巴西"),
    ("GIG", "巴西"),
    ("MEX", "墨西哥"),
    ("SCL", "智利"),
    ("EZE", "阿根廷"),
    ("JNB", "南非"),
    ("DXB", "阿联酋"),
    ("IST", "土耳其"),
    ("TLV", "以色列"),
    ("AMS", "荷兰"),
    ("ZRH", "瑞士"),
    ("ARN", "瑞典"),
    ("OSL", "挪威"),
    ("HEL", "芬兰"),
    ("CPH", "丹麦"),
    ("WAW", "波兰"),
    ("PRG", "捷克"),
    ("VIE", "奥地利"),
    ("BRU", "比利时"),
    ("DUB", "爱尔兰"),
    ("LIS", "葡萄牙"),
    ("MAD", "西班牙"),
    ("BCN", "西班牙"),
    ("FCO", "意大利"),
    ("MXP", "意大利"),
    ("OTP", "罗马尼亚"),
    ("BUD", "匈牙利"),
    ("SOF", "保加利亚"),
    ("ZAG", "克罗地亚"),
    ("LJU", "斯洛文尼亚"),
    ("BTS", "斯洛伐克"),
    ("VNO", "立陶宛"),
    ("RIX", "拉脱维亚"),
    ("TLL", "爱沙尼亚"),
    ("MLA", "马耳他"),
    ("LCA", "塞浦路斯"),
    ("LUX", "卢森堡"),
    ("KEF", "冰岛"),
    ("PEK", "中国"),
    ("PKX", "中国"),
    ("PVG", "中国"),
    ("SHA", "中国"),
    ("SZX", "中国"),
    ("CAN", "中国"),
    ("HKG", "香港"),
];

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

    /// 机场三字码不得与其他别名冲突。
    #[test]
    fn airport_codes_do_not_conflict_with_aliases() {
        for (code, _) in AIRPORT_CODES {
            let upper = code.to_ascii_uppercase();
            assert!(
                code_to_display(&upper).is_none() || code.len() > 3,
                "airport code {} collides with an ISO alias",
                code
            );
        }
    }
}
