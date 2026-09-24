import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AttachmentViewportScheduler } from "./attachmentViewportScheduler";

describe("AttachmentViewportScheduler", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("waits for dwell before probing and cancels if the card leaves early", async () => {
    const probed: string[] = [];
    const scheduler = new AttachmentViewportScheduler({
      dwellMs: 200,
      maxConcurrent: 2,
      onProbe: async (source) => {
        probed.push(source);
      },
    });

    scheduler.setVisible("c1", "/a.md", true);
    await vi.advanceTimersByTimeAsync(150);
    expect(probed).toEqual([]);
    expect(scheduler.getQueuedSources()).toEqual([]);

    scheduler.setVisible("c1", "/a.md", false);
    await vi.advanceTimersByTimeAsync(200);
    expect(probed).toEqual([]);

    scheduler.setVisible("c1", "/a.md", true);
    await vi.advanceTimersByTimeAsync(200);
    expect(probed).toEqual(["/a.md"]);
  });

  it("removes a queued source when the last interested card leaves before start", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const probed: string[] = [];
    const scheduler = new AttachmentViewportScheduler({
      dwellMs: 50,
      maxConcurrent: 1,
      onProbe: async (source) => {
        probed.push(source);
        if (source === "/busy.md") await gate;
      },
    });

    scheduler.setVisible("busy", "/busy.md", true);
    await vi.advanceTimersByTimeAsync(50);
    expect(probed).toEqual(["/busy.md"]);

    scheduler.setVisible("queued", "/queued.md", true);
    await vi.advanceTimersByTimeAsync(50);
    expect(scheduler.getQueuedSources()).toEqual(["/queued.md"]);

    scheduler.setVisible("queued", "/queued.md", false);
    expect(scheduler.getQueuedSources()).toEqual([]);

    release();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(0);
    expect(probed).toEqual(["/busy.md"]);
  });

  it("dedupes by source across cards and keeps interest until the last card leaves", async () => {
    const probed: string[] = [];
    const scheduler = new AttachmentViewportScheduler({
      dwellMs: 50,
      maxConcurrent: 2,
      onProbe: async (source) => {
        probed.push(source);
      },
    });

    scheduler.setVisible("c1", "/same.md", true);
    scheduler.setVisible("c2", "/same.md", true);
    await vi.advanceTimersByTimeAsync(50);
    expect(probed).toEqual(["/same.md"]);
    expect(scheduler.getInterest("/same.md")).toBe(2);

    scheduler.setVisible("c1", "/same.md", false);
    expect(scheduler.getInterest("/same.md")).toBe(1);
    scheduler.setVisible("c2", "/same.md", false);
    expect(scheduler.getInterest("/same.md")).toBe(0);
  });

  it("limits concurrency and drains the queue", async () => {
    let running = 0;
    let peak = 0;
    const releases: Array<() => void> = [];
    const scheduler = new AttachmentViewportScheduler({
      dwellMs: 10,
      maxConcurrent: 2,
      onProbe: async () => {
        running += 1;
        peak = Math.max(peak, running);
        await new Promise<void>((resolve) => releases.push(resolve));
        running -= 1;
      },
    });

    for (const id of ["a", "b", "c", "d"]) {
      scheduler.setVisible(id, `/${id}.md`, true);
    }
    await vi.advanceTimersByTimeAsync(10);
    expect(peak).toBe(2);
    expect(scheduler.getProbingSources()).toHaveLength(2);
    expect(scheduler.getQueuedSources()).toHaveLength(2);

    releases[0]!();
    releases[1]!();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(0);
    expect(scheduler.getProbingSources()).toHaveLength(2);

    releases[2]!();
    releases[3]!();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(0);
    expect(scheduler.getProbingSources()).toHaveLength(0);
    expect(scheduler.getQueuedSources()).toHaveLength(0);
  });

  it("requestImmediate bypasses dwell", async () => {
    const probed: string[] = [];
    const scheduler = new AttachmentViewportScheduler({
      dwellMs: 5000,
      onProbe: async (source) => {
        probed.push(source);
      },
    });
    scheduler.requestImmediate("/now.md");
    await Promise.resolve();
    expect(probed).toEqual(["/now.md"]);
  });

  it("cancelQueued drops waiting sources but leaves the API for in-flight", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const probed: string[] = [];
    const scheduler = new AttachmentViewportScheduler({
      dwellMs: 10,
      maxConcurrent: 1,
      onProbe: async (source) => {
        probed.push(source);
        if (source === "/in-flight.md") await gate;
      },
    });

    scheduler.setVisible("a", "/in-flight.md", true);
    await vi.advanceTimersByTimeAsync(10);
    scheduler.setVisible("b", "/queued.md", true);
    await vi.advanceTimersByTimeAsync(10);
    expect(scheduler.getQueuedSources()).toEqual(["/queued.md"]);

    scheduler.cancelQueued();
    expect(scheduler.getQueuedSources()).toEqual([]);
    expect(probed).toEqual(["/in-flight.md"]);

    release();
    await Promise.resolve();
  });
});
