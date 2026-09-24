import { useVirtualizer } from "@tanstack/react-virtual";
import { RefreshCw } from "lucide-react";
import {
  memo,
  useEffect,
  useLayoutEffect,
  type ReactNode,
  type RefObject,
} from "react";
import { useTranslation } from "react-i18next";
import type { LocalSessionMessage } from "../../../lib/types";
import styles from "../page.module.css";

/** Former CSS grid gap between messages — included in measured row height. */
export const SESSION_MESSAGE_ROW_GAP_PX = 12;
export const SESSION_MESSAGE_ESTIMATE_PX = 140;
export const SESSION_MESSAGE_OVERSCAN = 8;

type Props = {
  messages: LocalSessionMessage[];
  loading: boolean;
  /** Selected session id — resets scroll when it changes. */
  sessionKey: string;
  scrollRef: RefObject<HTMLDivElement | null>;
  renderMessage: (message: LocalSessionMessage, index: number) => ReactNode;
};

/**
 * Virtualized session transcript. Keeps `.sessionMessages` as the scroll
 * element so attachment IntersectionObservers (root = this node) keep working.
 * Rows unmount when far from the viewport; card useEffect cleanups unregister probes.
 */
export const VirtualSessionMessages = memo(function VirtualSessionMessages({
  messages,
  loading,
  sessionKey,
  scrollRef,
  renderMessage,
}: Props) {
  const { t } = useTranslation();
  const count = loading ? 0 : messages.length;

  const virtualizer = useVirtualizer({
    count,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => SESSION_MESSAGE_ESTIMATE_PX,
    overscan: SESSION_MESSAGE_OVERSCAN,
    getItemKey: (index) => {
      const message = messages[index];
      return `${sessionKey}:${message?.timestamp ?? "t"}:${index}`;
    },
  });

  // Preserve historical behavior: open / switch session at the top.
  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    element.scrollTop = 0;
  }, [sessionKey, scrollRef]);

  useEffect(() => {
    if (loading) return;
    const element = scrollRef.current;
    if (!element) return;
    element.scrollTop = 0;
  }, [loading, sessionKey, scrollRef]);

  if (loading) {
    return (
      <div className={styles.sessionDetailEmpty}>
        <RefreshCw aria-hidden="true" className={styles.spinning} size={24} />
        <p>{t("sessionsMessagesLoading")}</p>
      </div>
    );
  }

  if (!messages.length) {
    return (
      <div className={styles.sessionDetailEmpty}>
        <p>{t("sessionsMessagesEmpty")}</p>
      </div>
    );
  }

  const items = virtualizer.getVirtualItems();

  return (
    <div
      className={styles.sessionMessagesVirtual}
      style={{ height: virtualizer.getTotalSize() }}
    >
      {items.map((item) => {
        const message = messages[item.index];
        if (!message) return null;
        return (
          <div
            className={styles.sessionMessageRow}
            data-index={item.index}
            key={item.key}
            ref={virtualizer.measureElement}
            style={{ transform: `translateY(${item.start}px)` }}
          >
            {renderMessage(message, item.index)}
          </div>
        );
      })}
    </div>
  );
});
