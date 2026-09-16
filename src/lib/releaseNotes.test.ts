import { describe, expect, it } from "vitest";
import {
  hasReleaseNotes,
  parseInline,
  parseReleaseNotes,
  safeHref,
} from "./releaseNotes";

const SAMPLE = `### Added

- Official login browser picker (**Chrome** / Edge)
- Settings top tabs: drag to reorder

### Fixed

- Theme \`system\` follow-up
1. First ordered
2. Second ordered

See [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/).

[bad](javascript:alert(1))
`;

describe("safeHref", () => {
  it("allows http(s) only", () => {
    expect(safeHref("https://example.com/a")).toBe("https://example.com/a");
    expect(safeHref("http://example.com")).toBe("http://example.com/");
    expect(safeHref("javascript:alert(1)")).toBeUndefined();
    expect(safeHref("data:text/html,hi")).toBeUndefined();
    expect(safeHref("/relative")).toBeUndefined();
  });
});

describe("parseInline", () => {
  it("parses bold, code, and safe links", () => {
    const nodes = parseInline("Hi **bold** and `code` plus [docs](https://example.com)");
    expect(nodes).toEqual([
      { type: "text", value: "Hi " },
      { type: "bold", children: [{ type: "text", value: "bold" }] },
      { type: "text", value: " and " },
      { type: "code", value: "code" },
      { type: "text", value: " plus " },
      {
        type: "link",
        href: "https://example.com/",
        children: [{ type: "text", value: "docs" }],
      },
    ]);
  });

  it("strips unsafe link schemes to plain text label", () => {
    expect(parseInline("[x](javascript:alert(1))")).toEqual([
      { type: "text", value: "x" },
    ]);
  });
});

describe("parseReleaseNotes", () => {
  it("parses headings, lists, paragraphs from Keep a Changelog sample", () => {
    const blocks = parseReleaseNotes(SAMPLE);
    expect(blocks.map((b) => b.type)).toEqual([
      "heading",
      "ul",
      "heading",
      "ul",
      "ol",
      "paragraph",
      "paragraph",
    ]);

    const added = blocks[0];
    expect(added?.type).toBe("heading");
    if (added?.type === "heading") {
      expect(added.level).toBe(3);
      expect(added.children).toEqual([{ type: "text", value: "Added" }]);
    }

    const ul = blocks[1];
    expect(ul?.type).toBe("ul");
    if (ul?.type === "ul") {
      expect(ul.items).toHaveLength(2);
      expect(ul.items[0]).toEqual([
        { type: "text", value: "Official login browser picker (" },
        { type: "bold", children: [{ type: "text", value: "Chrome" }] },
        { type: "text", value: " / Edge)" },
      ]);
    }

    const fixedList = blocks[3];
    expect(fixedList?.type).toBe("ul");
    if (fixedList?.type === "ul") {
      expect(fixedList.items[0]).toEqual([
        { type: "text", value: "Theme " },
        { type: "code", value: "system" },
        { type: "text", value: " follow-up" },
      ]);
    }

    const ol = blocks[4];
    expect(ol?.type).toBe("ol");
    if (ol?.type === "ol") expect(ol.items).toHaveLength(2);

    const linkPara = blocks[5];
    expect(linkPara?.type).toBe("paragraph");
    if (linkPara?.type === "paragraph") {
      expect(linkPara.children.some((n) => n.type === "link")).toBe(true);
    }

    const bad = blocks[6];
    expect(bad?.type).toBe("paragraph");
    if (bad?.type === "paragraph") {
      expect(bad.children.every((n) => n.type !== "link")).toBe(true);
      expect(bad.children).toEqual([{ type: "text", value: "bad" }]);
    }
  });

  it("returns empty for blank notes", () => {
    expect(parseReleaseNotes("")).toEqual([]);
    expect(parseReleaseNotes("   \n\n  ")).toEqual([]);
    expect(hasReleaseNotes(undefined)).toBe(false);
    expect(hasReleaseNotes("")).toBe(false);
    expect(hasReleaseNotes(SAMPLE)).toBe(true);
  });
});
