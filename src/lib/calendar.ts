/** Local-timezone calendar helpers for relative day labels and absolute date-times. */

export function localDateKey(date = new Date()) {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

export function startOfLocalDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

/**
 * Whole local calendar days from `now` until `iso`.
 * - `undefined` if missing/invalid
 * - `-1` if the instant is already in the past
 * - `0` same local calendar day (still upcoming)
 * - `1` tomorrow, etc.
 */
export function calendarDaysUntil(iso?: string, nowMs: number = Date.now()): number | undefined {
  if (!iso) return undefined;
  const reset = new Date(iso);
  const resetMs = reset.getTime();
  if (Number.isNaN(resetMs)) return undefined;
  if (resetMs <= nowMs) return -1;
  const dayMs = 86_400_000;
  return Math.round((startOfLocalDay(reset) - startOfLocalDay(new Date(nowMs))) / dayMs);
}

/** Same as calendarDaysUntil, but accepts a Unix timestamp in seconds. */
export function calendarDaysUntilUnix(
  expiresAtSec?: number | null,
  nowMs: number = Date.now(),
): number | undefined {
  if (expiresAtSec == null || !Number.isFinite(expiresAtSec)) return undefined;
  return calendarDaysUntil(new Date(expiresAtSec * 1000).toISOString(), nowMs);
}

export type InstantSource = {
  iso?: string | null;
  unixSec?: number | null;
};

/** Resolve an ISO string or unix-seconds instant to a local Date. */
export function resolveInstant(source?: InstantSource | null): Date | undefined {
  if (!source) return undefined;
  if (source.iso) {
    const date = new Date(source.iso);
    return Number.isNaN(date.getTime()) ? undefined : date;
  }
  if (source.unixSec != null && Number.isFinite(source.unixSec)) {
    const date = new Date(source.unixSec * 1000);
    return Number.isNaN(date.getTime()) ? undefined : date;
  }
  return undefined;
}

function normalizeLocale(locale?: string) {
  if (!locale) return undefined;
  if (locale.startsWith("zh")) return "zh-CN";
  if (locale.startsWith("en")) return "en-US";
  return locale;
}

/**
 * Absolute local date-time, 24h, slash-separated.
 * Drops the year when it matches `now` (e.g. `10/12 08:00`); otherwise `2026/10/12 08:00`.
 */
export function formatLocalDateTime(
  date: Date,
  options?: { locale?: string; nowMs?: number },
): string {
  const locale = normalizeLocale(options?.locale);
  const nowYear = new Date(options?.nowMs ?? Date.now()).getFullYear();
  const parts = new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    hourCycle: "h23",
  }).formatToParts(date);
  const get = (type: Intl.DateTimeFormatPartTypes) =>
    parts.find((part) => part.type === type)?.value ?? "";
  const year = get("year");
  const month = get("month");
  const day = get("day");
  const hour = get("hour").padStart(2, "0");
  const minute = get("minute").padStart(2, "0");
  const datePart = Number(year) === nowYear ? `${month}/${day}` : `${year}/${month}/${day}`;
  return `${datePart} ${hour}:${minute}`;
}

/** Local 24h clock `HH:mm`. */
export function formatLocalTime(date: Date, locale?: string): string {
  const parts = new Intl.DateTimeFormat(normalizeLocale(locale), {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    hourCycle: "h23",
  }).formatToParts(date);
  const get = (type: Intl.DateTimeFormatPartTypes) =>
    parts.find((part) => part.type === type)?.value ?? "";
  return `${get("hour").padStart(2, "0")}:${get("minute").padStart(2, "0")}`;
}

export type TranslateFn = (key: string, options?: Record<string, unknown>) => string;

export type DatedRelativeLabel = {
  /** Compact badge text (today / tomorrow / Nd / expired). */
  short: string;
  /** Inline display with absolute date-time when available. */
  full: string;
  /** Absolute local date-time only (for tooltip/title). */
  absolute?: string;
  days?: number;
};

type DatedKind = "subscription" | "reset" | "quota";

/**
 * Build short + full relative labels for a future/past instant.
 * When no instant is available, falls back to day-count-only wording (no fabricated date).
 */
export function formatDatedRelative(options: {
  instant?: Date;
  /** Used when `instant` is missing (e.g. backend daysRemaining only). */
  daysFallback?: number;
  nowMs?: number;
  locale?: string;
  t: TranslateFn;
  kind: DatedKind;
}): DatedRelativeLabel {
  const nowMs = options.nowMs ?? Date.now();
  const { t, kind, locale } = options;
  const instant = options.instant;
  const days = instant
    ? calendarDaysUntil(instant.toISOString(), nowMs)
    : options.daysFallback;
  const absolute = instant ? formatLocalDateTime(instant, { locale, nowMs }) : undefined;
  const time = instant ? formatLocalTime(instant, locale) : undefined;

  if (days === undefined) {
    const unknown =
      kind === "subscription" ? t("subscriptionUnknownExpiry")
      : kind === "quota" ? t("quotaResetUnknown")
      : t("usageUnknown");
    return { short: unknown, full: unknown, days };
  }

  if (days === 0) {
    const short =
      kind === "subscription" ? t("subscriptionToday")
      : kind === "quota" ? t("quotaResetToday")
      : t("usageResetsToday");
    const full = time
      ? t(
          kind === "subscription" ? "subscriptionTodayAt"
          : kind === "quota" ? "quotaResetTodayAt"
          : "usageResetsTodayAt",
          { time },
        )
      : short;
    return { short, full, absolute, days };
  }

  if (days === 1) {
    const short =
      kind === "subscription" ? t("subscriptionTomorrow")
      : kind === "quota" ? t("quotaResetTomorrow")
      : t("usageResetsTomorrow");
    const full = time
      ? t(
          kind === "subscription" ? "subscriptionTomorrowAt"
          : kind === "quota" ? "quotaResetTomorrowAt"
          : "usageResetsTomorrowAt",
          { time },
        )
      : short;
    return { short, full, absolute, days };
  }

  if (days > 1) {
    const short =
      kind === "subscription" ? t("subscriptionDays", { count: days })
      : kind === "quota" ? t("quotaResetDays", { count: days })
      : t("usageResetsIn", { count: days });
    const full = absolute
      ? t(
          kind === "subscription" ? "subscriptionDaysWithDate"
          : kind === "quota" ? "quotaResetDaysWithDate"
          : "usageResetsInWithDate",
          { count: days, date: absolute },
        )
      : short;
    return { short, full, absolute, days };
  }

  // past — subscription expires; quota/reset windows just rolled over
  const short =
    kind === "subscription" ? t("subscriptionExpired")
    : kind === "quota" ? t("quotaResetPassed")
    : t("usageResetPassed");
  const full = absolute
    ? t(
        kind === "subscription" ? "subscriptionExpiredWithDate"
        : kind === "quota" ? "quotaResetPassedWithDate"
        : "usageResetPassedWithDate",
        { date: absolute },
      )
    : short;
  return { short, full, absolute, days };
}
