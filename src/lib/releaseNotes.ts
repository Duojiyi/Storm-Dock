/** Lightweight Keep-a-Changelog / GitHub Release notes → safe React-ready AST. No HTML injection. */

export type InlineNode =
  | { type: "text"; value: string }
  | { type: "bold"; children: InlineNode[] }
  | { type: "code"; value: string }
  | { type: "link"; href: string; children: InlineNode[] };

export type BlockNode =
  | { type: "heading"; level: 2 | 3; children: InlineNode[] }
  | { type: "paragraph"; children: InlineNode[] }
  | { type: "ul"; items: InlineNode[][] }
  | { type: "ol"; items: InlineNode[][] };

const SAFE_HREF = /^https?:\/\//i;

/** Allow only http(s) absolute URLs; reject javascript:, data:, etc. */
export function safeHref(href: string): string | undefined {
  const trimmed = href.trim();
  if (!SAFE_HREF.test(trimmed)) return undefined;
  try {
    const url = new URL(trimmed);
    if (url.protocol !== "http:" && url.protocol !== "https:") return undefined;
    return url.href;
  } catch {
    return undefined;
  }
}

/** Parse inline markdown: **bold**, `code`, [text](url). */
export function parseInline(text: string): InlineNode[] {
  const nodes: InlineNode[] = [];
  // Link target allows one level of nested parentheses (e.g. Wikipedia-style URLs).
  const re = /(\*\*(.+?)\*\*|`([^`]+)`|\[([^\]]+)\]\(((?:[^()]|\([^)]*\))+)\))/g;
  let last = 0;
  let match: RegExpExecArray | null;
  while ((match = re.exec(text)) !== null) {
    if (match.index > last) {
      nodes.push({ type: "text", value: text.slice(last, match.index) });
    }
    if (match[2] !== undefined) {
      nodes.push({ type: "bold", children: parseInline(match[2]) });
    } else if (match[3] !== undefined) {
      nodes.push({ type: "code", value: match[3] });
    } else {
      const href = safeHref(match[5] ?? "");
      const label = match[4] ?? "";
      if (href) {
        nodes.push({ type: "link", href, children: parseInline(label) });
      } else {
        // Unsafe / non-http(s): keep visible label only (no raw markdown junk).
        nodes.push({ type: "text", value: label });
      }
    }
    last = match.index + match[0].length;
  }
  if (last < text.length) nodes.push({ type: "text", value: text.slice(last) });
  return nodes.length ? nodes : [{ type: "text", value: "" }];
}

const HEADING = /^(#{2,3})\s+(.+)$/;
const UL_ITEM = /^[-*]\s+(.+)$/;
const OL_ITEM = /^(\d+)\.\s+(.+)$/;

/** Parse block-level release notes markdown into an AST. */
export function parseReleaseNotes(markdown: string): BlockNode[] {
  const lines = markdown.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n");
  const blocks: BlockNode[] = [];
  let i = 0;

  const flushParagraph = (buf: string[]) => {
    const text = buf.join(" ").trim();
    if (text) blocks.push({ type: "paragraph", children: parseInline(text) });
  };

  while (i < lines.length) {
    const raw = lines[i] ?? "";
    const line = raw.trimEnd();
    const trimmed = line.trim();

    if (!trimmed) {
      i += 1;
      continue;
    }

    const heading = HEADING.exec(trimmed);
    if (heading) {
      const level = (heading[1]?.length ?? 2) as 2 | 3;
      blocks.push({
        type: "heading",
        level: level === 3 ? 3 : 2,
        children: parseInline((heading[2] ?? "").trim()),
      });
      i += 1;
      continue;
    }

    const ul = UL_ITEM.exec(trimmed);
    if (ul) {
      const items: InlineNode[][] = [];
      while (i < lines.length) {
        const t = (lines[i] ?? "").trim();
        if (!t) break;
        const m = UL_ITEM.exec(t);
        if (!m) break;
        items.push(parseInline((m[1] ?? "").trim()));
        i += 1;
      }
      if (items.length) blocks.push({ type: "ul", items });
      continue;
    }

    const ol = OL_ITEM.exec(trimmed);
    if (ol) {
      const items: InlineNode[][] = [];
      while (i < lines.length) {
        const t = (lines[i] ?? "").trim();
        if (!t) break;
        const m = OL_ITEM.exec(t);
        if (!m) break;
        items.push(parseInline((m[2] ?? "").trim()));
        i += 1;
      }
      if (items.length) blocks.push({ type: "ol", items });
      continue;
    }

    const para: string[] = [trimmed];
    i += 1;
    while (i < lines.length) {
      const t = (lines[i] ?? "").trim();
      if (!t || HEADING.test(t) || UL_ITEM.test(t) || OL_ITEM.test(t)) break;
      para.push(t);
      i += 1;
    }
    flushParagraph(para);
  }

  return blocks;
}

export function hasReleaseNotes(markdown: string | undefined | null): boolean {
  if (!markdown) return false;
  return parseReleaseNotes(markdown).length > 0;
}
