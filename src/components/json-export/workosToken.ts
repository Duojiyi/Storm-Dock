/** Pull workos_token out of an export payload (object or one-item array). */
export function workosTokenFromExportData(data: unknown): string | undefined {
  const fromRecord = (value: unknown): string | undefined => {
    if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
    const token = (value as Record<string, unknown>).workos_token;
    return typeof token === "string" && token.length > 0 ? token : undefined;
  };

  const direct = fromRecord(data);
  if (direct) return direct;

  if (Array.isArray(data)) {
    for (const item of data) {
      const token = fromRecord(item);
      if (token) return token;
    }
  }
  return undefined;
}
