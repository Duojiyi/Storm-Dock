import { describe, expect, it } from "vitest";
import { productParts } from "./format";

describe("productParts", () => {
  it("joins grok product shares the way the details line shows them", () => {
    expect(productParts([
      { name: "Grok Build", percent: 96 },
      { name: "Imagine", percent: 4 },
    ])).toBe("Grok Build 96% + Imagine 4%");
  });
});
