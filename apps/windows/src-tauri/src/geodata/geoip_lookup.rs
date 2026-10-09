// src-tauri/src/geodata/geoip_lookup.rs
//! GeoIP.dat (v2ray protobuf) 同步解析与 IP→ISO 国家代码查找。
//! 文件约 17 MB，解析约 2000 条 v4 CIDR + 数百条 v6 CIDR，极快。

use std::fs::File;
use std::io::{BufReader, Read};
use std::net::IpAddr;
use std::path::Path;

use crate::util::error::{Error, Result};

type IpCidr = (Vec<u8>, u32);

/// 解析后的 GeoIP 索引：v4 与 v6 各自按起始 IP 排序的 (start, end, country_code) 向量。
#[derive(Debug, Clone, Default)]
pub struct GeoIpIndex {
    v4: Vec<(u32, u32, String)>,
    v6: Vec<(u128, u128, String)>,
}

impl GeoIpIndex {
    /// 从 GeoIP.dat 文件加载并构建索引。
    /// 预期耗时 ~100-300ms（17 MB，单线程），建议在 spawn_blocking 中调用。
    pub fn load(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mut reader = BufReader::with_capacity(128 * 1024, file);
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;

        // 验证最小长度与 protobuf 起始标签 (field 1 len-delimited = 0x0A)
        if buf.len() < 2 || buf[0] != 0x0A {
            return Err(Error::Other(
                "GeoIP.dat: not a v2ray protobuf file (missing field 1 tag)".into(),
            ));
        }

        let mut idx = Self::default();
        let mut i = 0;
        let n = buf.len();

        while i < n {
            // 读 varint tag
            let (tag, next_i) = Self::read_varint(&buf, i)?;
            i = next_i;
            let field = tag >> 3;
            let wt = tag & 7;

            if wt != 2 {
                // 跳过非 len-delimited 字段（reverse_match 等）
                // 这里不读长度，因为 tag 已包含 wiretype=0/1/5 的值在 tag 里（protobuf 格式）
                // 但实际上 wiretype 0/1/5 的值不在 tag 里，需要单独读取。
                // 为简化：只处理 field 1 (GeoIP 列表)，其余字段直接跳过其 payload。
                // 但标准 protobuf 对 wiretype 0/1/5 的值紧跟在 tag 后。
                // 此处如果遇到非 field 1 且 wt!=2，我们无法确定长度 -> 简单起见直接尝试按标准跳过：
                if wt == 0 {
                    let (_, ni) = Self::read_varint(&buf, i)?;
                    i = ni;
                } else if wt == 5 {
                    i += 4;
                } else if wt == 1 {
                    i += 8;
                } else {
                    // unknown wt, break
                    break;
                }
                continue;
            }

            // wt == 2: len-delimited
            let (len, ni) = Self::read_varint(&buf, i)?;
            i = ni;
            let sub_end = i + len as usize;
            if sub_end > n {
                return Err(Error::Other(
                    "GeoIP.dat: truncated len-delimited field".into(),
                ));
            }

            if field == 1 {
                // 解析单个 GeoIP entry
                let (code, cidrs) = Self::parse_entry(&buf[i..sub_end])?;
                if let Some(c) = code {
                    for (ip_bytes, prefix) in cidrs {
                        match ip_bytes.len() {
                            4 => {
                                let start = u32::from_be_bytes([
                                    ip_bytes[0],
                                    ip_bytes[1],
                                    ip_bytes[2],
                                    ip_bytes[3],
                                ]);
                                let mask = if prefix >= 32 {
                                    0xffffffff
                                } else {
                                    !0u32 << (32 - prefix)
                                };
                                let end = start | (!mask);
                                idx.v4.push((start, end, c.clone()));
                            }
                            16 => {
                                let start = u128::from_be_bytes(ip_bytes[..16].try_into().unwrap());
                                let mask = if prefix >= 128 {
                                    !0u128
                                } else {
                                    !0u128 << (128 - prefix)
                                };
                                let end = start | (!mask);
                                idx.v6.push((start, end, c.clone()));
                            }
                            _ => {} // 忽略异常长度
                        }
                    }
                }
            }

            i = sub_end;
        }

        idx.v4.sort_by_key(|r| r.0);
        idx.v6.sort_by_key(|r| r.0);
        Ok(idx)
    }

    /// 查找 IP 对应的 ISO 国家代码（不存在则返回 None）。
    pub fn lookup(&self, ip: IpAddr) -> Option<&str> {
        match ip {
            IpAddr::V4(ip) => {
                let ip = u32::from(ip);
                // 二分查找最后一个 start <= ip
                let mut lo = 0;
                let mut hi = self.v4.len();
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    if self.v4[mid].0 <= ip {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                if lo == 0 {
                    return None;
                }
                let (_s, e, c) = &self.v4[lo - 1];
                if ip <= *e {
                    Some(c.as_str())
                } else {
                    None
                }
            }
            IpAddr::V6(ip) => {
                let ip = u128::from(ip);
                let mut lo = 0;
                let mut hi = self.v6.len();
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    if self.v6[mid].0 <= ip {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                if lo == 0 {
                    return None;
                }
                let (_s, e, c) = &self.v6[lo - 1];
                if ip <= *e {
                    Some(c.as_str())
                } else {
                    None
                }
            }
        }
    }

    /// 读取 protobuf varint
    fn read_varint(buf: &[u8], mut i: usize) -> Result<(u64, usize)> {
        let mut r = 0u64;
        let mut shift = 0;
        loop {
            if i >= buf.len() {
                return Err(Error::Other("varint truncated".into()));
            }
            let x = buf[i];
            i += 1;
            r |= ((x & 0x7f) as u64) << shift;
            shift += 7;
            if (x & 0x80) == 0 {
                break;
            }
            if shift > 63 {
                return Err(Error::Other("varint too long".into()));
            }
        }
        Ok((r, i))
    }

    /// 解析单个 GeoIP entry (field 1 的 payload)
    fn parse_entry(buf: &[u8]) -> Result<(Option<String>, Vec<IpCidr>)> {
        let mut i = 0;
        let n = buf.len();
        let mut code = None;
        let mut cidrs = Vec::new();

        while i < n {
            let (tag, ni) = Self::read_varint(buf, i)?;
            i = ni;
            let field = tag >> 3;
            let wt = tag & 7;

            if wt != 2 {
                // 仅 field 1 string, field 2 len-delimited；其他跳过
                if wt == 0 {
                    let (_, ni) = Self::read_varint(buf, i)?;
                    i = ni;
                } else if wt == 5 {
                    i += 4;
                } else if wt == 1 {
                    i += 8;
                } else {
                    break;
                }
                continue;
            }

            let (len, ni) = Self::read_varint(buf, i)?;
            i = ni;
            let se = i + len as usize;
            if se > n {
                return Err(Error::Other("truncated entry".into()));
            }

            if field == 1 {
                code = Some(String::from_utf8_lossy(&buf[i..se]).to_string());
            } else if field == 2 {
                // 解析 CIDR 子消息
                let (cip, pfx) = Self::parse_cidr(&buf[i..se])?;
                cidrs.push((cip, pfx));
            }
            i = se;
        }
        Ok((code, cidrs))
    }

    fn parse_cidr(buf: &[u8]) -> Result<(Vec<u8>, u32)> {
        let mut i = 0;
        let n = buf.len();
        let mut ip = None;
        let mut pfx = None;
        while i < n {
            let (tag, ni) = Self::read_varint(buf, i)?;
            i = ni;
            let f = tag >> 3;
            let wt = tag & 7;
            if wt != 2 {
                if wt == 0 {
                    let (v, ni) = Self::read_varint(buf, i)?;
                    i = ni;
                    if f == 2 {
                        pfx = Some(v as u32);
                    }
                } else if wt == 5 {
                    i += 4;
                } else if wt == 1 {
                    i += 8;
                } else {
                    break;
                }
                continue;
            }
            let (len, ni) = Self::read_varint(buf, i)?;
            i = ni;
            let se = i + len as usize;
            if se > n {
                return Err(Error::Other("truncated cidr".into()));
            }
            if f == 1 {
                ip = Some(buf[i..se].to_vec());
            } else if f == 2 {
                // prefix is varint
                let (v, _) = Self::read_varint(buf, i)?;
                pfx = Some(v as u32);
            }
            i = se;
        }
        ip.zip(pfx)
            .ok_or_else(|| Error::Other("cidr missing ip or prefix".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手工构造一个最小合法 GeoIP protobuf：US 含 8.8.8.8/32, CN 含 114.114.114.114/32
    fn build_test_geoip() -> Vec<u8> {
        // 我们手工编码：
        // GeoIPList: field 1 repeated = 0x0A [len] entry...
        // entry: field 1 string "US", field 2 repeated CIDR
        // CIDR: field 1 bytes ip, field 2 varint prefix
        let mut out = Vec::new();
        // US entry
        let mut us = Vec::new();
        // country_code = "US" -> tag 0x0A, len 2, bytes "US"
        us.push(0x0A);
        us.push(0x02);
        us.extend_from_slice(b"US");
        // cidr: ip=8.8.8.8 (4 bytes), prefix=32
        let mut cidr = Vec::new();
        cidr.push(0x0A);
        cidr.push(0x04);
        cidr.extend_from_slice(&[8, 8, 8, 8]); // field 1 bytes
        cidr.push(0x10);
        cidr.push(0x20); // field 2 varint 32
                         // repeated cidr field 2 len-delimited
        us.push(0x12);
        us.push(cidr.len() as u8);
        us.extend(cidr);
        // entry len-delimited field 1
        out.push(0x0A);
        out.push(us.len() as u8);
        out.extend(us);
        // CN entry
        let mut cn = Vec::new();
        cn.push(0x0A);
        cn.push(0x02);
        cn.extend_from_slice(b"CN");
        let mut cidr2 = Vec::new();
        cidr2.push(0x0A);
        cidr2.push(0x04);
        cidr2.extend_from_slice(&[114, 114, 114, 114]);
        cidr2.push(0x10);
        cidr2.push(0x20);
        cn.push(0x12);
        cn.push(cidr2.len() as u8);
        cn.extend(cidr2);
        out.push(0x0A);
        out.push(cn.len() as u8);
        out.extend(cn);
        out
    }

    #[test]
    fn test_geoip_load_and_lookup() {
        let data = build_test_geoip();
        let tmp = std::env::temp_dir().join("test_geoip.dat");
        std::fs::write(&tmp, &data).unwrap();
        let idx = GeoIpIndex::load(&tmp).unwrap();
        assert_eq!(idx.lookup("8.8.8.8".parse().unwrap()), Some("US"));
        assert_eq!(idx.lookup("114.114.114.114".parse().unwrap()), Some("CN"));
        assert_eq!(idx.lookup("1.1.1.1".parse().unwrap()), None);
        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn test_varint() {
        assert_eq!(GeoIpIndex::read_varint(&[0x01], 0).unwrap(), (1, 1));
        assert_eq!(GeoIpIndex::read_varint(&[0x80, 0x01], 0).unwrap(), (128, 2));
        assert_eq!(
            GeoIpIndex::read_varint(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F], 0).unwrap(),
            (4294967295, 5)
        );
    }
}
