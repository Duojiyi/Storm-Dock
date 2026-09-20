import { describe, expect, it } from "vitest";
import { workosTokenFromExportData } from "./workosToken";

describe("workosTokenFromExportData", () => {
  it("reads from a record", () => {
    expect(workosTokenFromExportData({ workos_token: "abc", other: 1 })).toBe("abc");
  });

  it("reads from a one-item export array", () => {
    expect(workosTokenFromExportData([{ name: "x", workos_token: "tok" }])).toBe("tok");
  });

  it("returns undefined when missing or empty", () => {
    expect(workosTokenFromExportData({})).toBeUndefined();
    expect(workosTokenFromExportData([{ workos_token: "" }])).toBeUndefined();
    expect(workosTokenFromExportData(null)).toBeUndefined();
  });
});
