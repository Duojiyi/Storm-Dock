/** Editor-style pretty-print lines for JSON export preview. */

import { isSensitiveJsonKey } from "./sensitiveKeys";

export type JsonSegment =
  | { kind: "punct"; text: string }
  | { kind: "key"; text: string }
  | { kind: "string"; text: string }
  | { kind: "number"; text: string }
  | { kind: "boolean"; text: string }
  | { kind: "null"; text: string }
  | { kind: "sensitive"; path: string; value: string };

export type JsonLine = {
  id: string;
  depth: number;
  segments: JsonSegment[];
};

/**
 * Flatten a JSON value into pretty-print lines (2-space indent), matching what
 * programmers see in an editor: one logical line per entry, soft-wrapped by CSS.
 */
export function flattenJson(data: unknown): JsonLine[] {
  const lines: JsonLine[] = [];
  writeValue(lines, data, "", 0, undefined, true);
  return lines;
}

function writeValue(
  lines: JsonLine[],
  value: unknown,
  path: string,
  depth: number,
  key: string | undefined,
  isLast: boolean,
): void {
  const comma = isLast ? "" : ",";

  if (value !== null && typeof value === "object") {
    const isArray = Array.isArray(value);
    const open = isArray ? "[" : "{";
    const close = isArray ? "]" : "}";
    const entries = isArray
      ? (value as unknown[]).map((item, index) => [String(index), item] as const)
      : Object.entries(value as Record<string, unknown>);

    if (entries.length === 0) {
      lines.push({
        id: path || "$",
        depth,
        segments: [...keyPrefix(key), { kind: "punct", text: `${open}${close}${comma}` }],
      });
      return;
    }

    lines.push({
      id: `${path || "$"}${open}`,
      depth,
      segments: [...keyPrefix(key), { kind: "punct", text: open }],
    });

    entries.forEach(([entryKey, child], index) => {
      const childPath = isArray
        ? `${path}[${entryKey}]`
        : path
          ? `${path}.${entryKey}`
          : entryKey;
      const childKey = isArray ? undefined : entryKey;
      writeValue(lines, child, childPath, depth + 1, childKey, index === entries.length - 1);
    });

    lines.push({
      id: `${path || "$"}${close}`,
      depth,
      segments: [{ kind: "punct", text: `${close}${comma}` }],
    });
    return;
  }

  lines.push({
    id: path || "$",
    depth,
    segments: [...keyPrefix(key), ...valueSegments(value, path, key), ...(comma ? [{ kind: "punct" as const, text: comma }] : [])],
  });
}

function keyPrefix(key: string | undefined): JsonSegment[] {
  if (key === undefined) return [];
  return [
    { kind: "key", text: JSON.stringify(key) },
    { kind: "punct", text: ": " },
  ];
}

function valueSegments(value: unknown, path: string, key: string | undefined): JsonSegment[] {
  if (value === null) return [{ kind: "null", text: "null" }];
  if (typeof value === "boolean") return [{ kind: "boolean", text: value ? "true" : "false" }];
  if (typeof value === "number") {
    return Number.isFinite(value) ? [{ kind: "number", text: String(value) }] : [{ kind: "null", text: "null" }];
  }
  if (typeof value === "string") {
    if (isSensitiveJsonKey(key)) {
      return [{ kind: "sensitive", path, value }];
    }
    return [{ kind: "string", text: JSON.stringify(value) }];
  }
  return [{ kind: "string", text: JSON.stringify(value) }];
}
