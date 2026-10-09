import { describe, expect, it } from "vitest";
import { resolveGroupId, sortRuleGroups } from "./groups";

describe("sortRuleGroups", () => {
  it("drops GLOBAL and orders the five built-in groups canonically", () => {
    const input = ["自动优选", "GLOBAL", "影音视听", "扶梯出行", "人工优选", "人工智能"].map((name) => ({ name }));
    expect(sortRuleGroups(input).map((g) => g.name)).toEqual([
      "扶梯出行",
      "人工智能",
      "影音视听",
      "人工优选",
      "自动优选",
    ]);
  });

  it("puts custom groups last and does not mutate its input", () => {
    const input = [{ name: "自定义" }, { name: "扶梯出行" }];
    expect(sortRuleGroups(input).map((g) => g.name)).toEqual(["扶梯出行", "自定义"]);
    expect(input.map((g) => g.name)).toEqual(["自定义", "扶梯出行"]);
    expect(resolveGroupId("自定义")).toBe("自定义");
  });
});
