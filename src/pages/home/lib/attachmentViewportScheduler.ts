/** Viewport-driven attachment probe scheduler (DOM-free, unit-testable). */

export const ATTACHMENT_PROBE_DWELL_MS = 200;
export const ATTACHMENT_PROBE_MAX_CONCURRENT = 2;

export type AttachmentViewportSchedulerOptions = {
  dwellMs?: number;
  maxConcurrent?: number;
  /** Called when a source should start a network probe. Must settle; scheduler tracks concurrency. */
  onProbe: (source: string) => Promise<void>;
  setTimeoutFn?: typeof setTimeout;
  clearTimeoutFn?: typeof clearTimeout;
};

type CardState = {
  source: string;
  visible: boolean;
  /** Dwell completed while visible; counts toward queue interest. */
  armed: boolean;
  dwellTimer: ReturnType<typeof setTimeout> | undefined;
};

/**
 * Decides *when* to probe: dwell after entering the (near-)viewport, cancel if the
 * card leaves before the probe starts, dedupe by source, concurrency limit.
 * Cache hits should be handled by the caller before calling setVisible.
 */
export class AttachmentViewportScheduler {
  private readonly dwellMs: number;
  private readonly maxConcurrent: number;
  private readonly onProbe: (source: string) => Promise<void>;
  private readonly setTimeoutFn: typeof setTimeout;
  private readonly clearTimeoutFn: typeof clearTimeout;

  private cards = new Map<string, CardState>();
  /** Sources waiting to start (after dwell), FIFO. */
  private queue: string[] = [];
  private queued = new Set<string>();
  private probing = new Set<string>();
  /** Sources already probed in this session scope — do not re-queue after completion. */
  private done = new Set<string>();
  /** Armed visible cards interested in each source. */
  private interest = new Map<string, number>();
  private disposed = false;

  constructor(options: AttachmentViewportSchedulerOptions) {
    this.dwellMs = options.dwellMs ?? ATTACHMENT_PROBE_DWELL_MS;
    this.maxConcurrent = options.maxConcurrent ?? ATTACHMENT_PROBE_MAX_CONCURRENT;
    this.onProbe = options.onProbe;
    this.setTimeoutFn = options.setTimeoutFn ?? setTimeout;
    this.clearTimeoutFn = options.clearTimeoutFn ?? clearTimeout;
  }

  /** Test helpers */
  getQueuedSources(): string[] {
    return [...this.queue];
  }

  getProbingSources(): string[] {
    return [...this.probing];
  }

  getInterest(source: string): number {
    return this.interest.get(source) ?? 0;
  }

  setVisible(cardId: string, source: string, visible: boolean) {
    if (this.disposed) return;
    const key = source.trim();
    if (!key) return;

    let card = this.cards.get(cardId);
    if (!card) {
      card = { source: key, visible: false, armed: false, dwellTimer: undefined };
      this.cards.set(cardId, card);
    }

    if (card.source !== key) {
      this.clearDwell(card);
      if (card.armed) {
        card.armed = false;
        this.removeInterest(card.source);
        if ((this.interest.get(card.source) ?? 0) <= 0) {
          this.dequeue(card.source);
        }
      }
      card.source = key;
      card.visible = false;
    }

    if (visible === card.visible) return;
    card.visible = visible;

    if (visible) {
      this.clearDwell(card);
      if (this.done.has(key)) {
        return;
      }
      card.dwellTimer = this.setTimeoutFn(() => {
        card!.dwellTimer = undefined;
        if (this.disposed || !card!.visible) return;
        if (!card!.armed) {
          card!.armed = true;
          this.addInterest(card!.source);
          this.enqueue(card!.source);
          this.pump();
        }
      }, this.dwellMs) as ReturnType<typeof setTimeout>;
      return;
    }

    // Left viewport before/while waiting.
    this.clearDwell(card);
    if (card.armed) {
      card.armed = false;
      this.removeInterest(card.source);
      if ((this.interest.get(card.source) ?? 0) <= 0) {
        this.dequeue(card.source);
      }
    }
  }

  /** Bypass dwell — e.g. user clicked Download on an unchecked card. */
  requestImmediate(source: string) {
    if (this.disposed) return;
    const key = source.trim();
    if (!key) return;
    this.done.delete(key);
    // Ensure interest so leave of other cards won't dequeue mid-flight start.
    if ((this.interest.get(key) ?? 0) <= 0) {
      this.addInterest(key);
    }
    this.enqueue(key);
    this.pump();
  }

  unregister(cardId: string) {
    const card = this.cards.get(cardId);
    if (!card) return;
    this.clearDwell(card);
    if (card.armed) {
      card.armed = false;
      this.removeInterest(card.source);
      if ((this.interest.get(card.source) ?? 0) <= 0) {
        this.dequeue(card.source);
      }
    }
    this.cards.delete(cardId);
  }

  /** Drop queued-but-not-started probes (session switch). In-flight keep running. */
  cancelQueued() {
    this.queue = [];
    this.queued.clear();
    this.done.clear();
    for (const card of this.cards.values()) {
      this.clearDwell(card);
      if (card.armed) {
        card.armed = false;
      }
      card.visible = false;
    }
    this.interest.clear();
  }

  dispose() {
    this.cancelQueued();
    this.disposed = true;
    this.cards.clear();
  }

  private enqueue(source: string) {
    if (this.done.has(source) || this.queued.has(source) || this.probing.has(source)) return;
    this.queued.add(source);
    this.queue.push(source);
  }

  private dequeue(source: string) {
    if (!this.queued.has(source)) return;
    this.queued.delete(source);
    this.queue = this.queue.filter((item) => item !== source);
  }

  private addInterest(source: string) {
    this.interest.set(source, (this.interest.get(source) ?? 0) + 1);
  }

  private removeInterest(source: string) {
    const next = (this.interest.get(source) ?? 0) - 1;
    if (next <= 0) this.interest.delete(source);
    else this.interest.set(source, next);
  }

  private clearDwell(card: CardState) {
    if (card.dwellTimer !== undefined) {
      this.clearTimeoutFn(card.dwellTimer);
      card.dwellTimer = undefined;
    }
  }

  private pump() {
    while (this.probing.size < this.maxConcurrent && this.queue.length > 0) {
      const source = this.queue.shift()!;
      this.queued.delete(source);
      if ((this.interest.get(source) ?? 0) <= 0) {
        continue;
      }
      this.probing.add(source);
      void this.onProbe(source)
        .catch(() => {
          /* caller handles errors via its own promise */
        })
        .finally(() => {
          this.probing.delete(source);
          this.done.add(source);
          this.pump();
        });
    }
  }
}
