import { describe, expect, it } from "vitest";
import { flattenJson } from "./flattenJson";

function textOf(data: unknown): string {
  return flattenJson(data)
    .map((line) => {
      const indent = "  ".repeat(line.depth);
      const body = line.segments
        .map((seg) => {
          if (seg.kind === "sensitive") return JSON.stringify(seg.value);
          return seg.text;
        })
        .join("");
      return indent + body;
    })
    .join("\n");
}

describe("flattenJson", () => {
  it("matches JSON.stringify pretty print for plain data", () => {
    const data = {
      name: "storm",
      count: 2,
      ok: true,
      empty: null,
      nested: { a: 1 },
      list: ["x", "y"],
    };
    expect(textOf(data)).toBe(JSON.stringify(data, null, 2));
  });

  it("marks sensitive string values", () => {
    const lines = flattenJson({ access_token: "secret-token-value", other: "ok" });
    const tokenLine = lines.find((line) => line.segments.some((s) => s.kind === "sensitive"));
    expect(tokenLine).toBeTruthy();
    const sensitive = tokenLine!.segments.find((s) => s.kind === "sensitive");
    expect(sensitive).toMatchObject({ kind: "sensitive", value: "secret-token-value", path: "access_token" });
  });

  it("keeps empty containers on one line", () => {
    expect(textOf({ a: {}, b: [] })).toBe(JSON.stringify({ a: {}, b: [] }, null, 2));
  });
});
