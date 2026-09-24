import { describe, expect, it } from "vitest";
import {
  calendarDaysUntil,
  calendarDaysUntilUnix,
  formatDatedRelative,
  formatLocalDateTime,
  formatLocalTime,
  localDateKey,
  resolveInstant,
  startOfLocalDay,
} from "./calendar";

describe("calendarDaysUntil", () => {
  it("labels an overnight reset as tomorrow (reported bug vs elapsed-ms/24h)", () => {
    // Local calendar: now Sep 23 20:00, reset Sep 24 02:42 → tomorrow
    const now = new Date(2026, 8, 23, 20, 0, 0).getTime();
    const reset = new Date(2026, 8, 24, 2, 42, 0).toISOString();
    expect(calendarDaysUntil(reset, now)).toBe(1);
    // Elapsed 24h buckets incorrectly call this "today"
    expect(Math.floor((Date.parse(reset) - now) / 86_400_000)).toBe(0);
  });

  it("labels a same-day upcoming reset as today", () => {
    const now = new Date(2026, 8, 24, 1, 0, 0).getTime();
    const reset = new Date(2026, 8, 24, 2, 42, 0).toISOString();
    expect(calendarDaysUntil(reset, now)).toBe(0);
  });

  it("returns -1 after the reset instant on the same local day", () => {
    const now = new Date(2026, 8, 24, 10, 0, 0).getTime();
    const reset = new Date(2026, 8, 24, 2, 42, 0).toISOString();
    expect(calendarDaysUntil(reset, now)).toBe(-1);
  });

  it("uses local calendar days even when both instants share a UTC date", () => {
    const now = new Date(2026, 8, 23, 20, 0, 0).getTime();
    const resetDate = new Date(2026, 8, 24, 2, 42, 0);
    const reset = resetDate.toISOString();
    expect(calendarDaysUntil(reset, now)).toBe(1);
    // Document UTC-date collision for Asia/Shanghai-like offsets (UTC+8..+14)
    const offsetHours = -new Date(2026, 8, 23).getTimezoneOffset() / 60;
    if (offsetHours >= 8) {
      expect(new Date(now).getUTCFullYear()).toBe(2026);
      expect(new Date(now).getUTCMonth()).toBe(8);
      expect(new Date(now).getUTCDate()).toBe(23);
      expect(resetDate.getUTCDate()).toBe(23);
    }
  });

  it("counts multi-day gaps by local calendar", () => {
    const now = new Date(2026, 8, 23, 20, 0, 0).getTime();
    const reset = new Date(2026, 8, 26, 2, 42, 0).toISOString();
    expect(calendarDaysUntil(reset, now)).toBe(3);
  });

  it("returns undefined for missing/invalid input", () => {
    expect(calendarDaysUntil(undefined)).toBeUndefined();
    expect(calendarDaysUntil("not-a-date")).toBeUndefined();
  });
});

describe("localDateKey", () => {
  it("formats YYYY-MM-DD in local time", () => {
    const date = new Date(2026, 8, 23, 20, 0, 0);
    expect(localDateKey(date)).toBe("2026-09-23");
    expect(startOfLocalDay(date)).toBe(new Date(2026, 8, 23).getTime());
  });
});


describe("calendarDaysUntilUnix", () => {
  it("matches overnight tomorrow semantics for unix seconds", () => {
    const now = new Date(2026, 8, 23, 20, 0, 0).getTime();
    const resetSec = Math.floor(new Date(2026, 8, 24, 2, 42, 0).getTime() / 1000);
    expect(calendarDaysUntilUnix(resetSec, now)).toBe(1);
  });

  it("handles month-end local calendar rollover", () => {
    const now = new Date(2026, 0, 31, 22, 0, 0).getTime(); // Jan 31
    const reset = new Date(2026, 1, 1, 2, 0, 0).toISOString(); // Feb 1
    expect(calendarDaysUntil(reset, now)).toBe(1);
    expect(calendarDaysUntilUnix(Math.floor(new Date(2026, 1, 1, 2, 0, 0).getTime() / 1000), now)).toBe(1);
  });
});


const tZh = (key: string, options?: Record<string, unknown>) => {
  const map: Record<string, string> = {
    subscriptionToday: "今天",
    subscriptionTomorrow: "明天",
    subscriptionDays: `剩余 ${options?.count} 天`,
    subscriptionExpired: "已过期",
    subscriptionUnknownExpiry: "--",
    subscriptionTodayAt: `今天 ${options?.time}`,
    subscriptionTomorrowAt: `明天 ${options?.time}`,
    subscriptionDaysWithDate: `剩余 ${options?.count} 天 · ${options?.date}`,
    subscriptionExpiredWithDate: `已过期 · ${options?.date}`,
    usageResetsToday: "今天重置",
    usageResetsTomorrow: "明天重置",
    usageResetsIn: `将在 ${options?.count} 天后重置`,
    usageResetPassed: "重置日期已过",
    usageUnknown: "--",
    usageResetsTodayAt: `今天 ${options?.time}`,
    usageResetsTomorrowAt: `明天 ${options?.time}`,
    usageResetsInWithDate: `将在 ${options?.count} 天后重置 · ${options?.date}`,
    usageResetPassedWithDate: `重置日期已过 · ${options?.date}`,
    quotaResetToday: "今天",
    quotaResetTomorrow: "明天",
    quotaResetDays: `${options?.count} 天`,
    quotaResetPassed: "额度已重置",
    quotaResetUnknown: "--",
    quotaResetTodayAt: `额度重置 · 今天 ${options?.time}`,
    quotaResetTomorrowAt: `额度重置 · 明天 ${options?.time}`,
    quotaResetDaysWithDate: `额度重置 · 剩余 ${options?.count} 天 · ${options?.date}`,
    quotaResetPassedWithDate: `额度已重置 · ${options?.date}`,
  };
  return map[key] ?? `${key}:${options?.count ?? ""}`;
};

const tEn = (key: string, options?: Record<string, unknown>) => {
  const map: Record<string, string> = {
    subscriptionToday: "today",
    subscriptionTomorrow: "tomorrow",
    subscriptionDays: `${options?.count} days`,
    subscriptionExpired: "Expired",
    subscriptionUnknownExpiry: "--",
    subscriptionTodayAt: `Today ${options?.time}`,
    subscriptionTomorrowAt: `Tomorrow ${options?.time}`,
    subscriptionDaysWithDate: `${options?.count} days left · ${options?.date}`,
    subscriptionExpiredWithDate: `Expired · ${options?.date}`,
    usageResetsToday: "Resets today",
    usageResetsTomorrow: "Resets tomorrow",
    usageResetsIn: `Resets in ${options?.count} days`,
    usageResetPassed: "Reset date has passed",
    usageUnknown: "--",
    usageResetsTodayAt: `Today ${options?.time}`,
    usageResetsTomorrowAt: `Tomorrow ${options?.time}`,
    usageResetsInWithDate: `Resets in ${options?.count} days · ${options?.date}`,
    usageResetPassedWithDate: `Reset date has passed · ${options?.date}`,
    quotaResetToday: "today",
    quotaResetTomorrow: "tomorrow",
    quotaResetDays: `${options?.count}d`,
    quotaResetPassed: "Quota reset",
    quotaResetUnknown: "--",
    quotaResetTodayAt: `Quota resets · Today ${options?.time}`,
    quotaResetTomorrowAt: `Quota resets · Tomorrow ${options?.time}`,
    quotaResetDaysWithDate: `Quota resets · ${options?.count} days left · ${options?.date}`,
    quotaResetPassedWithDate: `Quota reset · ${options?.date}`,
  };
  return map[key] ?? `${key}:${options?.count ?? ""}`;
};

describe("formatLocalDateTime / formatLocalTime", () => {
  it("formats 24h local and drops the current year", () => {
    const now = new Date(2026, 8, 23, 20, 0, 0).getTime();
    const date = new Date(2026, 9, 12, 8, 0, 0);
    expect(formatLocalDateTime(date, { nowMs: now, locale: "zh" })).toBe("10/12 08:00");
    expect(formatLocalTime(date, "zh")).toBe("08:00");
  });

  it("keeps the year when it differs from now", () => {
    const now = new Date(2026, 8, 23, 20, 0, 0).getTime();
    const date = new Date(2027, 0, 5, 14, 30, 0);
    expect(formatLocalDateTime(date, { nowMs: now, locale: "en" })).toBe("2027/01/05 14:30");
  });
});

describe("resolveInstant", () => {
  it("accepts ISO and unix seconds", () => {
    const iso = new Date(2026, 8, 24, 2, 42, 0).toISOString();
    expect(resolveInstant({ iso })?.getTime()).toBe(Date.parse(iso));
    const sec = Math.floor(Date.parse(iso) / 1000);
    expect(resolveInstant({ unixSec: sec })?.getTime()).toBe(sec * 1000);
    expect(resolveInstant({})).toBeUndefined();
  });
});

describe("formatDatedRelative", () => {
  const overnightNow = new Date(2026, 8, 23, 20, 0, 0).getTime();
  const overnight = new Date(2026, 8, 24, 2, 42, 0);
  const sameDayNow = new Date(2026, 8, 24, 1, 0, 0).getTime();
  const later = new Date(2026, 9, 12, 8, 0, 0);
  const past = new Date(2026, 8, 20, 10, 0, 0);

  it("zh: today / tomorrow embed the clock time", () => {
    expect(formatDatedRelative({
      instant: overnight, nowMs: sameDayNow, locale: "zh", t: tZh, kind: "subscription",
    })).toMatchObject({ short: "今天", full: "今天 02:42", days: 0 });
    expect(formatDatedRelative({
      instant: overnight, nowMs: overnightNow, locale: "zh", t: tZh, kind: "subscription",
    })).toMatchObject({ short: "明天", full: "明天 02:42", days: 1 });
  });

  it("en: today / tomorrow embed the clock time", () => {
    expect(formatDatedRelative({
      instant: overnight, nowMs: sameDayNow, locale: "en", t: tEn, kind: "subscription",
    }).full).toBe("Today 02:42");
    expect(formatDatedRelative({
      instant: overnight, nowMs: overnightNow, locale: "en", t: tEn, kind: "subscription",
    }).full).toBe("Tomorrow 02:42");
  });

  it("zh/en: N days and expired append absolute date-time", () => {
    expect(formatDatedRelative({
      instant: later, nowMs: overnightNow, locale: "zh", t: tZh, kind: "subscription",
    }).full).toBe("剩余 19 天 · 10/12 08:00");
    expect(formatDatedRelative({
      instant: past, nowMs: overnightNow, locale: "zh", t: tZh, kind: "subscription",
    }).full).toBe("已过期 · 09/20 10:00");
    expect(formatDatedRelative({
      instant: later, nowMs: overnightNow, locale: "en", t: tEn, kind: "subscription",
    }).full).toBe("19 days left · 10/12 08:00");
    expect(formatDatedRelative({
      instant: past, nowMs: overnightNow, locale: "en", t: tEn, kind: "subscription",
    }).full).toBe("Expired · 09/20 10:00");
  });

  it("reset variant uses reset copy with absolute date", () => {
    expect(formatDatedRelative({
      instant: overnight, nowMs: overnightNow, locale: "zh", t: tZh, kind: "reset",
    }).full).toBe("明天 02:42");
    expect(formatDatedRelative({
      instant: later, nowMs: overnightNow, locale: "zh", t: tZh, kind: "reset",
    }).full).toBe("将在 19 天后重置 · 10/12 08:00");
  });

  it("day-count fallback does not fabricate a date", () => {
    expect(formatDatedRelative({
      daysFallback: 5, nowMs: overnightNow, locale: "zh", t: tZh, kind: "subscription",
    })).toEqual({ short: "剩余 5 天", full: "剩余 5 天", days: 5 });
  });
});


describe("formatDatedRelative quota (Codex rate-limit)", () => {
  const overnightNow = new Date(2026, 8, 23, 20, 0, 0).getTime();
  const overnight = new Date(2026, 8, 24, 2, 42, 0);
  const later = new Date(2026, 8, 27, 8, 0, 0);
  const past = new Date(2026, 8, 20, 10, 0, 0);

  it("zh: prefixes 额度重置 and never says 已过期 for a past window", () => {
    expect(formatDatedRelative({
      instant: overnight, nowMs: overnightNow, locale: "zh", t: tZh, kind: "quota",
    }).full).toBe("额度重置 · 明天 02:42");
    expect(formatDatedRelative({
      instant: later, nowMs: overnightNow, locale: "zh", t: tZh, kind: "quota",
    }).full).toBe("额度重置 · 剩余 4 天 · 09/27 08:00");
    const pastLabel = formatDatedRelative({
      instant: past, nowMs: overnightNow, locale: "zh", t: tZh, kind: "quota",
    });
    expect(pastLabel.short).toBe("额度已重置");
    expect(pastLabel.full).toBe("额度已重置 · 09/20 10:00");
    expect(pastLabel.full).not.toContain("已过期");
    expect(pastLabel.short).not.toContain("已过期");
  });

  it("en: uses Quota resets / Quota reset", () => {
    expect(formatDatedRelative({
      instant: overnight, nowMs: overnightNow, locale: "en", t: tEn, kind: "quota",
    }).full).toBe("Quota resets · Tomorrow 02:42");
    const pastLabel = formatDatedRelative({
      instant: past, nowMs: overnightNow, locale: "en", t: tEn, kind: "quota",
    });
    expect(pastLabel.short).toBe("Quota reset");
    expect(pastLabel.full).not.toMatch(/Expired/i);
  });
});
