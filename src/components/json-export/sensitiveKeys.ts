/** Keys whose string values collapse by default in export previews. */
const SENSITIVE = new Set([
  "access_token",
  "accessToken",
  "refresh_token",
  "refreshToken",
  "workos_token",
  "WorkosCursorSessionToken",
]);

export function isSensitiveJsonKey(key: string | undefined): boolean {
  return key !== undefined && SENSITIVE.has(key);
}
