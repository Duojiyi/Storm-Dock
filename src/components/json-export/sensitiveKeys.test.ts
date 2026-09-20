import { describe, expect, it } from "vitest";
import { isSensitiveJsonKey } from "./sensitiveKeys";

describe("isSensitiveJsonKey", () => {
  it("matches export token fields", () => {
    expect(isSensitiveJsonKey("access_token")).toBe(true);
    expect(isSensitiveJsonKey("refreshToken")).toBe(true);
    expect(isSensitiveJsonKey("refresh_token")).toBe(true);
    expect(isSensitiveJsonKey("workos_token")).toBe(true);
  });

  it("ignores ordinary fields", () => {
    expect(isSensitiveJsonKey("email")).toBe(false);
    expect(isSensitiveJsonKey(undefined)).toBe(false);
  });
});
