import type { AttachmentAvailability } from "../../../lib/types";

export const ATTACHMENT_EXPIRED_PREFIX = "attachment_expired:";
export const ATTACHMENT_EXPIRED_STORAGE_KEY = "storm-dock.session-attachment-expired.v1";
const MAX_CONCURRENT_CHECKS = 2;

const memoryCache = new Map<string, AttachmentAvailability>();
let activeChecks = 0;
/** Bumped when the selected session changes; queued probes from older generations are dropped. */
let probeGeneration = 0;
type Waiter = { generation: number; resolve: () => void };
const waitQueue: Waiter[] = [];

export type AttachmentProbeResult =
  | { kind: "result"; availability: AttachmentAvailability }
  | { kind: "cancelled" };

let persistedExpired: Set<string> | undefined;
let persistTimer: ReturnType<typeof setTimeout> | undefined;
const PERSIST_DEBOUNCE_MS = 400;

function loadPersistedExpired(): Set<string> {
  if (persistedExpired) return persistedExpired;
  persistedExpired = new Set();
  if (typeof localStorage === "undefined") return persistedExpired;
  try {
    const raw = localStorage.getItem(ATTACHMENT_EXPIRED_STORAGE_KEY);
    if (!raw) return persistedExpired;
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return persistedExpired;
    for (const item of parsed) {
      if (typeof item === "string" && item.length > 0) persistedExpired.add(item);
    }
  } catch {
    /* ignore corrupt storage */
  }
  return persistedExpired;
}

function schedulePersistExpired() {
  if (typeof localStorage === "undefined" || !persistedExpired) return;
  if (persistTimer !== undefined) clearTimeout(persistTimer);
  const snapshot = [...persistedExpired];
  const write = () => {
    persistTimer = undefined;
    try {
      localStorage.setItem(ATTACHMENT_EXPIRED_STORAGE_KEY, JSON.stringify(snapshot));
    } catch {
      /* quota / private mode — memory cache still works */
    }
  };
  if (typeof requestIdleCallback === "function") {
    persistTimer = setTimeout(() => {
      requestIdleCallback(() => write(), { timeout: 1000 });
    }, PERSIST_DEBOUNCE_MS) as ReturnType<typeof setTimeout>;
  } else {
    persistTimer = setTimeout(write, PERSIST_DEBOUNCE_MS) as ReturnType<typeof setTimeout>;
  }
}

function persistExpired(source: string) {
  const set = loadPersistedExpired();
  if (set.has(source)) return;
  set.add(source);
  schedulePersistExpired();
}

export function resetAttachmentAvailabilityCacheForTests() {
  memoryCache.clear();
  activeChecks = 0;
  probeGeneration = 0;
  waitQueue.length = 0;
  persistedExpired = undefined;
  if (persistTimer !== undefined) {
    clearTimeout(persistTimer);
    persistTimer = undefined;
  }
  if (typeof localStorage !== "undefined") {
    localStorage.removeItem(ATTACHMENT_EXPIRED_STORAGE_KEY);
  }
}

/**
 * Start a new probe scope for the currently selected session.
 * Cancels queued-but-not-started probes from the previous scope.
 * In-flight probes may still finish and populate the cache, but callers
 * with the old generation treat the result as cancelled for UI updates.
 */
export function beginAttachmentProbeScope(sessionId: string | undefined): number {
  probeGeneration += 1;
  const pending = waitQueue.splice(0);
  for (const waiter of pending) {
    waiter.resolve();
  }
  void sessionId;
  return probeGeneration;
}

export function getAttachmentProbeGeneration(): number {
  return probeGeneration;
}

export function getCachedAttachmentAvailability(source: string): AttachmentAvailability | undefined {
  const key = source.trim();
  if (!key) return undefined;
  const cached = memoryCache.get(key);
  if (cached) return cached;
  if (loadPersistedExpired().has(key)) {
    const expired: AttachmentAvailability = { status: "expired", reason: "链接已失效" };
    memoryCache.set(key, expired);
    return expired;
  }
  return undefined;
}

export function cacheAttachmentAvailability(source: string, availability: AttachmentAvailability) {
  const key = source.trim();
  if (!key) return;
  if (availability.status === "unavailable") {
    // Transient — do not keep around for the whole session.
    memoryCache.delete(key);
    return;
  }
  memoryCache.set(key, availability);
  if (availability.status === "expired") {
    persistExpired(key);
  }
}

export function markAttachmentExpired(source: string, reason?: string) {
  cacheAttachmentAvailability(source, {
    status: "expired",
    reason: reason?.trim() || "链接已失效",
  });
}

export function isExpiredAttachmentError(error: unknown): boolean {
  const message = errorMessage(error);
  return (
    message.includes(ATTACHMENT_EXPIRED_PREFIX) ||
    message.includes("远程沙箱中找不到该附件") ||
    /\bnot[_ ]?found\b/i.test(message)
  );
}

export function formatAttachmentErrorMessage(error: unknown): string {
  const message = errorMessage(error);
  const stripped = message.includes(ATTACHMENT_EXPIRED_PREFIX)
    ? message.slice(message.indexOf(ATTACHMENT_EXPIRED_PREFIX) + ATTACHMENT_EXPIRED_PREFIX.length)
    : message;
  return stripped.trim() || message;
}

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error && typeof (error as { message: unknown }).message === "string") {
    return (error as { message: string }).message;
  }
  return String(error ?? "");
}

async function withConcurrencyLimit<T>(
  generation: number,
  run: () => Promise<T>,
): Promise<T | undefined> {
  if (activeChecks >= MAX_CONCURRENT_CHECKS) {
    await new Promise<void>((resolve) => waitQueue.push({ generation, resolve }));
  }
  // Dropped while waiting — do not start the network probe.
  if (generation !== probeGeneration) {
    return undefined;
  }
  activeChecks += 1;
  try {
    if (generation !== probeGeneration) {
      return undefined;
    }
    return await run();
  } finally {
    activeChecks -= 1;
    const next = waitQueue.shift();
    if (next) next.resolve();
  }
}

/** Lazy availability lookup with in-memory + persisted-expired caching. */
export async function resolveAttachmentAvailability(
  source: string,
  check: (source: string) => Promise<AttachmentAvailability>,
  options?: { generation?: number },
): Promise<AttachmentProbeResult> {
  const key = source.trim();
  const generation = options?.generation ?? probeGeneration;
  if (!key) {
    return { kind: "result", availability: { status: "unavailable", reason: "无效的附件路径。" } };
  }
  if (generation !== probeGeneration) {
    return { kind: "cancelled" };
  }
  const cached = getCachedAttachmentAvailability(key);
  if (cached) return { kind: "result", availability: cached };

  const availability = await withConcurrencyLimit(generation, async () => {
    const again = getCachedAttachmentAvailability(key);
    if (again) return again;
    try {
      const next = await check(key);
      // Always cache durable results from in-flight work, even if the UI scope moved on.
      cacheAttachmentAvailability(key, next);
      if (next.status === "unavailable") {
        return next;
      }
      return getCachedAttachmentAvailability(key) ?? next;
    } catch (error) {
      if (isExpiredAttachmentError(error)) {
        const expired: AttachmentAvailability = {
          status: "expired",
          reason: formatAttachmentErrorMessage(error),
        };
        cacheAttachmentAvailability(key, expired);
        return expired;
      }
      const unavailable: AttachmentAvailability = {
        status: "unavailable",
        reason: formatAttachmentErrorMessage(error) || "暂时无法检查附件。",
      };
      return unavailable;
    }
  });

  if (availability === undefined || generation !== probeGeneration) {
    return { kind: "cancelled" };
  }
  return { kind: "result", availability };
}
