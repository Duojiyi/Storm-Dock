import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AttachmentAvailability } from "../../../lib/types";
import {
  ATTACHMENT_EXPIRED_PREFIX,
  beginAttachmentProbeScope,
  cacheAttachmentAvailability,
  formatAttachmentErrorMessage,
  getCachedAttachmentAvailability,
  isExpiredAttachmentError,
  markAttachmentExpired,
  resetAttachmentAvailabilityCacheForTests,
  resolveAttachmentAvailability,
} from "./attachmentAvailability";

describe("attachment availability helpers", () => {
  beforeEach(() => {
    resetAttachmentAvailabilityCacheForTests();
  });

  it("detects expired invoke errors and strips the stable prefix", () => {
    expect(isExpiredAttachmentError(`${ATTACHMENT_EXPIRED_PREFIX}远程沙箱中找不到该附件`)).toBe(true);
    expect(isExpiredAttachmentError("远程沙箱中找不到该附件（会话结束后文件可能已被清理）。")).toBe(true);
    expect(isExpiredAttachmentError("Error: not_found")).toBe(true);
    expect(isExpiredAttachmentError("需要已登录的 Grok Bot 账号")).toBe(false);
    expect(formatAttachmentErrorMessage(`${ATTACHMENT_EXPIRED_PREFIX}链接已失效`)).toBe("链接已失效");
  });

  it("persists expired results and skips re-checking them", async () => {
    markAttachmentExpired("file:///home/box/agent-data/agents/a/attachments/x.md");
    expect(getCachedAttachmentAvailability("file:///home/box/agent-data/agents/a/attachments/x.md")?.status).toBe(
      "expired",
    );
    const check = vi.fn(async () => ({ status: "available" }) as AttachmentAvailability);
    const result = await resolveAttachmentAvailability(
      "file:///home/box/agent-data/agents/a/attachments/x.md",
      check,
    );
    expect(result).toEqual({ kind: "result", availability: { status: "expired", reason: "链接已失效" } });
    expect(check).not.toHaveBeenCalled();
  });

  it("caches available results in memory but not unavailable ones", async () => {
    const check = vi
      .fn()
      .mockResolvedValueOnce({ status: "available" } satisfies AttachmentAvailability)
      .mockResolvedValueOnce({ status: "unavailable", reason: "网络错误" } satisfies AttachmentAvailability)
      .mockResolvedValueOnce({ status: "unavailable", reason: "网络错误" } satisfies AttachmentAvailability);

    await resolveAttachmentAvailability("/tmp/a.png", check);
    await resolveAttachmentAvailability("/tmp/a.png", check);
    expect(check).toHaveBeenCalledTimes(1);

    cacheAttachmentAvailability("/tmp/b.png", { status: "unavailable", reason: "auth" });
    expect(getCachedAttachmentAvailability("/tmp/b.png")).toBeUndefined();

    await resolveAttachmentAvailability("/tmp/b.png", check);
    await resolveAttachmentAvailability("/tmp/b.png", check);
    expect(check).toHaveBeenCalledTimes(3);
  });

  it("limits concurrent probes", async () => {
    let running = 0;
    let peak = 0;
    const check = vi.fn(async () => {
      running += 1;
      peak = Math.max(peak, running);
      await new Promise((resolve) => setTimeout(resolve, 20));
      running -= 1;
      return { status: "available" } satisfies AttachmentAvailability;
    });
    await Promise.all([
      resolveAttachmentAvailability("/tmp/c1", check),
      resolveAttachmentAvailability("/tmp/c2", check),
      resolveAttachmentAvailability("/tmp/c3", check),
      resolveAttachmentAvailability("/tmp/c4", check),
    ]);
    expect(peak).toBeLessThanOrEqual(2);
    expect(check).toHaveBeenCalledTimes(4);
  });

  it("cancels queued probes when the selected session scope changes", async () => {
    const started: string[] = [];
    const check = vi.fn(async (source: string) => {
      started.push(source);
      await new Promise((resolve) => setTimeout(resolve, 40));
      return { status: "available" } satisfies AttachmentAvailability;
    });

    const scopeA = beginAttachmentProbeScope("session-a");
    const probes = [
      resolveAttachmentAvailability("/tmp/a1", check, { generation: scopeA }),
      resolveAttachmentAvailability("/tmp/a2", check, { generation: scopeA }),
      resolveAttachmentAvailability("/tmp/a3", check, { generation: scopeA }),
      resolveAttachmentAvailability("/tmp/a4", check, { generation: scopeA }),
    ];

    // Let the concurrency slots fill (max 2), leave the rest queued.
    await new Promise((resolve) => setTimeout(resolve, 5));
    const scopeB = beginAttachmentProbeScope("session-b");
    const afterSwitch = await resolveAttachmentAvailability("/tmp/b1", check, { generation: scopeB });

    const results = await Promise.all(probes);
    const cancelled = results.filter((result) => result.kind === "cancelled");
    const completed = results.filter((result) => result.kind === "result");

    // Queued-but-not-started A probes must be cancelled; in-flight A may still finish.
    expect(cancelled.length).toBeGreaterThanOrEqual(1);
    expect(completed.length + cancelled.length).toBe(4);
    expect(afterSwitch.kind).toBe("result");
    expect(started).toContain("/tmp/b1");
    // Cancelled A probes must not have started after the scope bump.
    expect(started.filter((source) => source.startsWith("/tmp/a")).length).toBeLessThanOrEqual(2);
  });

  it("lets in-flight probes populate cache without updating a cancelled caller", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const check = vi.fn(async () => {
      await gate;
      return { status: "expired", reason: "gone" } satisfies AttachmentAvailability;
    });

    const scopeA = beginAttachmentProbeScope("session-a");
    const pending = resolveAttachmentAvailability("/tmp/inflight.md", check, { generation: scopeA });
    await new Promise((resolve) => setTimeout(resolve, 5));
    beginAttachmentProbeScope("session-b");
    release();
    const result = await pending;
    expect(result.kind).toBe("cancelled");
    expect(getCachedAttachmentAvailability("/tmp/inflight.md")?.status).toBe("expired");
  });
});
