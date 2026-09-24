import { Download, FileText, Image as ImageIcon, Paperclip } from "lucide-react";
import { memo, useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import { Tooltip } from "../../../components/Tooltip";
import { exportSessionAttachment, openExternalUrl, readSessionAttachmentPreview } from "../../../lib/api";
import type { AttachmentAvailability, SessionAttachment } from "../../../lib/types";
import {
  formatAttachmentErrorMessage,
  getCachedAttachmentAvailability,
  isExpiredAttachmentError,
  markAttachmentExpired,
} from "../lib/attachmentAvailability";
import { observeAttachmentInViewport, probeAttachmentNow } from "../lib/attachmentViewportProbe";
import styles from "../page.module.css";

/** Skip inline preview when the known size is large (decode cost). */
const MAX_INLINE_PREVIEW_BYTES = 512 * 1024;

function formatSize(size: number | undefined, t: (key: string, options?: Record<string, unknown>) => string) {
  if (size == null || !Number.isFinite(size) || size < 0) return undefined;
  if (size < 1024) return t("sessionAttachmentBytes", { count: size });
  if (size < 1024 * 1024) return t("sessionAttachmentKilobytes", { count: Math.round(size / 1024) });
  return t("sessionAttachmentMegabytes", { count: (size / (1024 * 1024)).toFixed(1) });
}

function isImageMime(mime: string | undefined, name: string) {
  if (mime?.startsWith("image/")) return true;
  return /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(name);
}

function attachmentSource(attachment: SessionAttachment) {
  return attachment.path || attachment.url || "";
}

function extensionLabel(attachment: SessionAttachment) {
  const fromName = attachment.name.includes(".") ? attachment.name.split(".").pop() : undefined;
  if (fromName) return fromName.toUpperCase();
  if (attachment.mime?.includes("/")) return attachment.mime.split("/")[1]!.toUpperCase();
  return "FILE";
}

type Props = {
  sessionId: string;
  probeGeneration: number;
  scrollRootRef: RefObject<HTMLElement | null>;
  attachments?: SessionAttachment[];
  onError: (error: unknown) => void;
  onNotice: (message: string) => void;
};

export const SessionAttachments = memo(function SessionAttachments({
  sessionId,
  probeGeneration,
  scrollRootRef,
  attachments,
  onError,
  onNotice,
}: Props) {
  const items = attachments?.filter((item) => item.name.trim()) ?? [];
  if (!items.length) return null;
  return (
    <div className={styles.sessionAttachments}>
      {items.map((attachment, index) => (
        <SessionAttachmentCard
          attachment={attachment}
          key={`${sessionId}:${attachment.id ?? attachment.name}-${index}`}
          onError={onError}
          onNotice={onNotice}
          probeGeneration={probeGeneration}
          scrollRootRef={scrollRootRef}
          sessionId={sessionId}
        />
      ))}
    </div>
  );
});

const SessionAttachmentCard = memo(function SessionAttachmentCard({
  sessionId,
  probeGeneration,
  scrollRootRef,
  attachment,
  onError,
  onNotice,
}: {
  sessionId: string;
  probeGeneration: number;
  scrollRootRef: RefObject<HTMLElement | null>;
  attachment: SessionAttachment;
  onError: (error: unknown) => void;
  onNotice: (message: string) => void;
}) {
  const { t } = useTranslation();
  const cardRef = useRef<HTMLDivElement>(null);
  const source = useMemo(() => attachmentSource(attachment), [attachment]);
  const image = isImageMime(attachment.mime, attachment.name);
  const [preview, setPreview] = useState<string>();
  const [downloading, setDownloading] = useState(false);
  const [availability, setAvailability] = useState<AttachmentAvailability | undefined>(() =>
    source ? getCachedAttachmentAvailability(source) : undefined,
  );
  const [checking, setChecking] = useState(false);
  const sizeLabel = formatSize(attachment.size, t);
  const typeLabel = attachment.mime || extensionLabel(attachment);
  const expired = availability?.status === "expired";
  const unavailable = availability?.status === "unavailable";
  const available = availability?.status === "available";

  useEffect(() => {
    if (expired) setPreview(undefined);
  }, [expired]);

  useEffect(() => {
    const root = scrollRootRef.current;
    const element = cardRef.current;
    if (!root || !element || !sessionId) return;
    if (!source) {
      setAvailability({ status: "unavailable", reason: "无效的附件路径。" });
      setChecking(false);
      return;
    }
    return observeAttachmentInViewport(root, element, source, sessionId, probeGeneration, (update) => {
      setChecking(update.checking);
      if (update.availability) setAvailability(update.availability);
    });
  }, [sessionId, source, probeGeneration, scrollRootRef]);

  useEffect(() => {
    if (!image || !source || !available || expired) return;
    if (attachment.size != null && attachment.size > MAX_INLINE_PREVIEW_BYTES) return;
    let cancelled = false;
    void readSessionAttachmentPreview(source)
      .then((value) => {
        if (!cancelled && value) setPreview(value);
      })
      .catch(() => {
        /* preview is best-effort */
      });
    return () => {
      cancelled = true;
    };
  }, [image, source, available, expired, attachment.size]);

  const ensureProbed = async (): Promise<AttachmentAvailability | undefined> => {
    if (availability?.status === "expired" || availability?.status === "available") {
      return availability;
    }
    const cached = source ? getCachedAttachmentAvailability(source) : undefined;
    if (cached) {
      setAvailability(cached);
      return cached;
    }
    if (!source) return undefined;
    setChecking(true);
    try {
      const result = await probeAttachmentNow(scrollRootRef.current, source, sessionId, probeGeneration);
      if (result.kind === "cancelled") return availability;
      setAvailability(result.availability);
      return result.availability;
    } finally {
      setChecking(false);
    }
  };

  const download = async () => {
    if (!source || downloading || expired || checking) return;
    if (/^https?:\/\//i.test(source)) {
      try {
        await openExternalUrl(source);
        onNotice(t("sessionAttachmentOpened"));
      } catch (error) {
        onError(error);
      }
      return;
    }

    const status = await ensureProbed();
    if (status?.status === "expired") {
      onNotice(t("sessionAttachmentExpired"));
      return;
    }

    const destination = await save({
      defaultPath: attachment.name,
      title: t("sessionAttachmentSaveTitle"),
    });
    if (!destination) return;
    setDownloading(true);
    try {
      await exportSessionAttachment(source, destination);
      onNotice(t("sessionAttachmentSaved"));
    } catch (error) {
      if (isExpiredAttachmentError(error)) {
        markAttachmentExpired(source, formatAttachmentErrorMessage(error));
        setAvailability({
          status: "expired",
          reason: formatAttachmentErrorMessage(error),
        });
        onNotice(t("sessionAttachmentExpired"));
      } else {
        onError(error);
      }
    } finally {
      setDownloading(false);
    }
  };

  const actionButton = (() => {
    if (checking) {
      return (
        <button
          aria-label={t("sessionAttachmentChecking")}
          className={`${styles.sessionAttachmentDownload} ${styles.sessionAttachmentChecking}`}
          disabled
          type="button"
        >
          {t("sessionAttachmentChecking")}
        </button>
      );
    }
    if (expired) {
      return (
        <button
          aria-label={t("sessionAttachmentExpired")}
          className={`${styles.sessionAttachmentDownload} ${styles.sessionAttachmentExpired}`}
          disabled
          title={availability?.reason}
          type="button"
        >
          {t("sessionAttachmentExpired")}
        </button>
      );
    }
    const button = (
      <button
        aria-label={t("sessionAttachmentDownload", { name: attachment.name })}
        className={styles.sessionAttachmentDownload}
        disabled={!source || downloading}
        onClick={() => void download()}
        type="button"
      >
        <Download aria-hidden="true" size={15} />
        {downloading
          ? t("sessionAttachmentSaving")
          : unavailable
            ? t("sessionAttachmentUnavailable")
            : t("download")}
      </button>
    );
    if (unavailable && availability?.reason) {
      return <Tooltip content={availability.reason}>{button}</Tooltip>;
    }
    return button;
  })();

  return (
    <div className={styles.sessionAttachment} ref={cardRef}>
      {preview && available ? (
        <button
          aria-label={t("sessionAttachmentPreview", { name: attachment.name })}
          className={styles.sessionAttachmentPreview}
          onClick={() => void download()}
          type="button"
        >
          <img alt={attachment.name} decoding="async" loading="lazy" src={preview} />
        </button>
      ) : (
        <div aria-hidden="true" className={styles.sessionAttachmentIcon}>
          {image ? <ImageIcon size={18} /> : <FileText size={18} />}
        </div>
      )}
      <div className={styles.sessionAttachmentMeta}>
        <strong title={attachment.name}>{attachment.name}</strong>
        <span>
          <Paperclip aria-hidden="true" size={11} />
          {typeLabel}
          {sizeLabel ? ` · ${sizeLabel}` : ""}
        </span>
      </div>
      {actionButton}
    </div>
  );
});
