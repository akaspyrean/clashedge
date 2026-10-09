import { describe, expect, it } from "vitest";
import { regionOf } from "./region";

describe("regionOf", () => {
  it.each([
    ["香港 02 IEPL", "hk"],
    ["HK-01", "hk"],
    ["HK01", "hk"],
    ["日本 东京 01", "jp"],
    ["JP | Osaka", "jp"],
    ["新加坡 02", "sg"],
    ["US-LA-03", "us"],
    ["美国 洛杉矶 01", "us"],
    ["台湾 01", "tw"],
    ["韩国 首尔", "kr"],
    ["UK London", "uk"],
    ["德国 法兰克福", "de"],
  ])("%s → %s", (name, id) => expect(regionOf(name)).toBe(id));

  it("does not mistake letters inside other words for a country code", () => {
    expect(regionOf("USB hub")).toBe("other");
    expect(regionOf("DIRECT")).toBe("other");
    expect(regionOf("自动优选")).toBe("other");
  });
});
