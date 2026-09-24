import { checkSessionAttachmentAvailability } from "../../../lib/api";
import type { AttachmentAvailability } from "../../../lib/types";
import {
  getCachedAttachmentAvailability,
  resolveAttachmentAvailability,
  type AttachmentProbeResult,
} from "./attachmentAvailability";
import {
  ATTACHMENT_PROBE_DWELL_MS,
  ATTACHMENT_PROBE_MAX_CONCURRENT,
  AttachmentViewportScheduler,
} from "./attachmentViewportScheduler";

export const ATTACHMENT_PROBE_ROOT_MARGIN = "200px 0px";

type AvailabilityListener = (update: {
  checking: boolean;
  availability?: AttachmentAvailability;
}) => void;

type CardRegistration = {
  source: string;
  listener: AvailabilityListener;
  generation: number;
};

type RootSession = {
  root: Element;
  generation: number;
  sessionId: string;
  observer: IntersectionObserver;
  scheduler: AttachmentViewportScheduler;
  cards: Map<Element, { id: string; registration: CardRegistration }>;
  inflight: Map<string, Promise<AttachmentProbeResult>>;
  nextCardId: number;
};

const sessions = new Map<Element, RootSession>();

function notify(reg: CardRegistration, checking: boolean, availability?: AttachmentAvailability) {
  reg.listener({ checking, availability });
}

function runProbe(session: RootSession, source: string): Promise<AttachmentProbeResult> {
  const existing = session.inflight.get(source);
  if (existing) return existing;

  const cached = getCachedAttachmentAvailability(source);
  if (cached) {
    return Promise.resolve({ kind: "result", availability: cached });
  }

  const promise = resolveAttachmentAvailability(source, checkSessionAttachmentAvailability, {
    generation: session.generation,
  }).finally(() => {
    session.inflight.delete(source);
  });
  session.inflight.set(source, promise);
  return promise;
}

function broadcastSource(
  session: RootSession,
  source: string,
  checking: boolean,
  availability?: AttachmentAvailability,
) {
  for (const entry of session.cards.values()) {
    if (entry.registration.source === source && entry.registration.generation === session.generation) {
      notify(entry.registration, checking, availability);
    }
  }
}

export function ensureViewportProbeSession(
  root: Element,
  sessionId: string,
  generation: number,
): RootSession {
  let session = sessions.get(root);
  if (session && (session.generation !== generation || session.sessionId !== sessionId)) {
    disposeViewportProbeSession(root);
    session = undefined;
  }
  if (session) return session;

  const scheduler = new AttachmentViewportScheduler({
    dwellMs: ATTACHMENT_PROBE_DWELL_MS,
    maxConcurrent: ATTACHMENT_PROBE_MAX_CONCURRENT,
    onProbe: async (source) => {
      const current = sessions.get(root);
      if (!current || current.generation !== generation) return;
      broadcastSource(current, source, true);
      const result = await runProbe(current, source);
      const latest = sessions.get(root);
      if (!latest || latest.generation !== generation) return;
      if (result.kind === "cancelled") {
        broadcastSource(current, source, false);
        return;
      }
      broadcastSource(current, source, false, result.availability);
    },
  });

  const observer =
    typeof IntersectionObserver === "undefined"
      ? (null as unknown as IntersectionObserver)
      : new IntersectionObserver(
          (entries) => {
            const current = sessions.get(root);
            if (!current) return;
            for (const entry of entries) {
              const card = current.cards.get(entry.target);
              if (!card) continue;
              current.scheduler.setVisible(card.id, card.registration.source, entry.isIntersecting);
            }
          },
          { root, rootMargin: ATTACHMENT_PROBE_ROOT_MARGIN, threshold: 0 },
        );

  session = {
    root,
    generation,
    sessionId,
    observer,
    scheduler,
    cards: new Map(),
    inflight: new Map(),
    nextCardId: 1,
  };
  sessions.set(root, session);
  return session;
}

export function disposeViewportProbeSession(root: Element | null | undefined) {
  if (!root) return;
  const session = sessions.get(root);
  if (!session) return;
  session.observer?.disconnect?.();
  session.scheduler.dispose();
  sessions.delete(root);
}

export function observeAttachmentInViewport(
  root: Element,
  element: Element,
  source: string,
  sessionId: string,
  generation: number,
  listener: AvailabilityListener,
): () => void {
  const key = source.trim();
  if (!key) {
    listener({ checking: false, availability: { status: "unavailable", reason: "无效的附件路径。" } });
    return () => undefined;
  }

  const cached = getCachedAttachmentAvailability(key);
  if (cached) {
    listener({ checking: false, availability: cached });
  } else {
    listener({ checking: false });
  }

  const session = ensureViewportProbeSession(root, sessionId, generation);
  const id = `card-${session.nextCardId++}`;
  session.cards.set(element, { id, registration: { source: key, listener, generation } });

  if (session.observer) {
    session.observer.observe(element);
  } else {
    // No IntersectionObserver (tests / ancient webview): treat as immediately visible.
    session.scheduler.setVisible(id, key, true);
  }

  return () => {
    const current = sessions.get(root);
    if (!current) return;
    current.observer?.unobserve?.(element);
    current.scheduler.unregister(id);
    current.cards.delete(element);
  };
}

/** Immediate probe for Download clicks — bypasses dwell, dedupes by path. */
export async function probeAttachmentNow(
  root: Element | null | undefined,
  source: string,
  sessionId: string,
  generation: number,
): Promise<AttachmentProbeResult> {
  const key = source.trim();
  if (!key) {
    return { kind: "result", availability: { status: "unavailable", reason: "无效的附件路径。" } };
  }
  const cached = getCachedAttachmentAvailability(key);
  if (cached) return { kind: "result", availability: cached };

  if (!root) {
    return resolveAttachmentAvailability(key, checkSessionAttachmentAvailability, { generation });
  }

  const session = ensureViewportProbeSession(root, sessionId, generation);
  broadcastSource(session, key, true);
  const result = await runProbe(session, key);
  if (result.kind === "result") {
    broadcastSource(session, key, false, result.availability);
  } else {
    broadcastSource(session, key, false);
  }
  return result;
}

/** @internal */
export function resetViewportProbeSessionsForTests() {
  for (const root of [...sessions.keys()]) {
    disposeViewportProbeSession(root);
  }
}
