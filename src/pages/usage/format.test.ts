import { describe, expect, it } from "vitest";
import { daysUntil, productParts, resetDatedLabel } from "./format";

const t = (key: string, options?: Record<string, unknown>) => {
  if (key === "usageResetsTodayAt") return `今天 ${options?.time}`;
  if (key === "usageResetsTomorrowAt") return `明天 ${options?.time}`;
  if (key === "usageResetsInWithDate") return `将在 ${options?.count} 天后重置 · ${options?.date}`;
  if (key === "usageResetPassedWithDate") return `重置日期已过 · ${options?.date}`;
  return `${key}:${options?.count ?? ""}`;
};

describe("productParts", () => {
  it("joins grok product shares the way the details line shows them", () => {
    expect(productParts([
      { name: "Grok Build", percent: 96 },
      { name: "Imagine", percent: 4 },
    ])).toBe("Grok Build 96% + Imagine 4%");
  });
});

describe("daysUntil (local calendar)", () => {
  it("maps overnight reset to 1 (tomorrow), not 0 (today)", () => {
    const now = new Date(2026, 8, 23, 20, 0, 0).getTime();
    const reset = new Date(2026, 8, 24, 2, 42, 0).toISOString();
    expect(daysUntil(reset, now)).toBe(1);
  });

  it("maps same-day upcoming reset to 0 (today)", () => {
    const now = new Date(2026, 8, 24, 1, 0, 0).getTime();
    const reset = new Date(2026, 8, 24, 2, 42, 0).toISOString();
    expect(daysUntil(reset, now)).toBe(0);
  });

  it("maps month-end overnight to 1 (tomorrow)", () => {
    const now = new Date(2026, 0, 31, 22, 0, 0).getTime();
    const reset = new Date(2026, 1, 1, 2, 0, 0).toISOString();
    expect(daysUntil(reset, now)).toBe(1);
  });
});

describe("resetDatedLabel", () => {
  it("shows tomorrow with clock inline", () => {
    const now = new Date(2026, 8, 23, 20, 0, 0).getTime();
    const reset = new Date(2026, 8, 24, 2, 42, 0).toISOString();
    const label = resetDatedLabel(reset, t, now);
    expect(label.short).toBe("usageResetsTomorrow:");
    expect(label.full).toBe("明天 02:42");
    expect(label.absolute).toMatch(/02:42/);
  });

  it("shows N-days with absolute date-time", () => {
    const now = new Date(2026, 8, 23, 20, 0, 0).getTime();
    const reset = new Date(2026, 9, 12, 8, 0, 0).toISOString();
    expect(resetDatedLabel(reset, t, now).full).toBe("将在 19 天后重置 · 10/12 08:00");
  });
});
