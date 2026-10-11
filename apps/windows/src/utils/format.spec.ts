import { describe, expect, it } from "vitest";
import { formatBytes, formatRate, isValidCidr, parseUserinfo } from "./format";

describe("formatBytes", () => {
  it("handles zero / invalid / negative", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(-5)).toBe("0 B");
    expect(formatBytes(Number.NaN)).toBe("0 B");
  });
  it("scales units and trims decimals for big values", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1024)).toBe("1.00 KB");
    expect(formatBytes(15 * 1024)).toBe("15.0 KB");
    expect(formatBytes(150 * 1024 * 1024)).toBe("150 MB");
    expect(formatBytes(5 * 1024 ** 4)).toBe("5.00 TB");
  });
});

describe("parseUserinfo", () => {
  it("parses a Subscription-Userinfo header", () => {
    expect(parseUserinfo("upload=10; download=20; total=100; expire=1700000000")).toEqual({
      used: 30,
      total: 100,
      expire: 1700000000,
    });
  });
  it("distinguishes 'never expires' (expire=0) from 'not provided'", () => {
    expect(parseUserinfo("download=1; total=2; expire=0")?.expire).toBe(0);
    expect(parseUserinfo("download=1; total=2")?.expire).toBeUndefined();
  });
  it("returns null for empty / unusable input", () => {
    expect(parseUserinfo(null)).toBeNull();
    expect(parseUserinfo("")).toBeNull();
    expect(parseUserinfo("garbage")).toBeNull();
    expect(parseUserinfo("expire=1")).toBeNull();
  });
});

describe("isValidCidr", () => {
  it("accepts valid IPv4 CIDR and IPv6 prefixes", () => {
    expect(isValidCidr("192.168.1.0/24")).toBe(true);
    expect(isValidCidr("0.0.0.0/0")).toBe(true);
    expect(isValidCidr("fe80::/10")).toBe(true);
  });
  it("rejects malformed entries", () => {
    expect(isValidCidr("192.168.1.0")).toBe(false);
    expect(isValidCidr("192.168.1.0/33")).toBe(false);
    expect(isValidCidr("256.1.1.1/8")).toBe(false);
    expect(isValidCidr("abc")).toBe(false);
  });
});

describe("formatRate", () => {
  it("appends /s to the byte formatting", () => {
    expect(formatRate(0)).toBe("0 B/s");
    expect(formatRate(1024 * 1024 * 4.82)).toBe("4.82 MB/s");
  });
});

describe("splitRate", () => {
  it("splits value and unit for big-number layouts", async () => {
    const { splitRate } = await import("./format");
    expect(splitRate(1024 * 1024 * 4.82)).toEqual(["4.82", "MB/s"]);
    expect(splitRate(0)).toEqual(["0", "B/s"]);
  });
});

describe("formatAppVersion", () => {
  it("stamps a v prefix to match release tags, keeps dev as-is", async () => {
    const { formatAppVersion } = await import("./format");
    expect(formatAppVersion("0.1.0")).toBe("v0.1.0");
    expect(formatAppVersion("v0.2.0")).toBe("v0.2.0");
    expect(formatAppVersion("dev")).toBe("dev");
    expect(formatAppVersion("")).toBe("");
  });
});

describe("maskUrl", () => {
  it("keeps structure and query keys, masking token-like segments and values", async () => {
    const { maskUrl } = await import("./format");
    // query 值全遮、键名保留
    expect(maskUrl("https://example.com/x?token=secret")).toBe("https://example.com/x?token=••••••••");
    // 末段形似 token（≥8 且含数字）→ 遮蔽；普通路径词 / 文件名保留
    expect(maskUrl("https://example.com/sub/Ab12CdEf34Gh")).toBe("https://example.com/sub/••••••••");
    expect(maskUrl("https://example.com/profiles/me.yaml")).toBe("https://example.com/profiles/me.yaml");
    expect(maskUrl("https://example.com/subscribe")).toBe("https://example.com/subscribe");
    expect(maskUrl("https://example.com")).toBe("https://example.com");
    // 幂等：已脱敏地址再过一遍不变
    expect(maskUrl("https://example.com/sub/••••••••")).toBe("https://example.com/sub/••••••••");
    expect(maskUrl("https://example.com/…")).toBe("https://example.com/…");
    // 解析失败不透出原文
    expect(maskUrl("not a url")).toBe("…");
  });
});
