import { ChevronDown, ChevronRight } from "lucide-react";
import {
  createContext,
  memo,
  useCallback,
  useContext,
  useMemo,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { CopyIconButton } from "../CopyIconButton";
import { Tooltip } from "../Tooltip";
import { flattenJson, type JsonLine, type JsonSegment } from "./flattenJson";
import styles from "./JsonExportPreview.module.css";

type ExpandApi = {
  isExpanded: (path: string) => boolean;
  toggle: (path: string) => void;
};

const ExpandContext = createContext<ExpandApi | null>(null);

function useExpandApi(): ExpandApi {
  const api = useContext(ExpandContext);
  if (!api) throw new Error("JsonExportPreview expand context missing");
  return api;
}

type JsonExportPreviewProps = {
  data: unknown;
};

/**
 * Programmer-style JSON preview: pretty-printed lines with 2-space indent.
 * Soft wrap keeps continuation under the first character of the line content
 * (field name for entries). Sensitive tokens collapse with hover copy/expand.
 */
export const JsonExportPreview = memo(function JsonExportPreview({ data }: JsonExportPreviewProps) {
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set());

  const toggle = useCallback((path: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const api = useMemo<ExpandApi>(
    () => ({
      isExpanded: (path) => expanded.has(path),
      toggle,
    }),
    [expanded, toggle],
  );

  const lines = useMemo(() => flattenJson(data), [data]);

  return (
    <ExpandContext.Provider value={api}>
      <div className={styles.preview} role="region">
        {lines.map((line) => (
          <JsonLineView key={line.id} line={line} />
        ))}
      </div>
    </ExpandContext.Provider>
  );
});

const JsonLineView = memo(function JsonLineView({ line }: { line: JsonLine }) {
  const style = useMemo(
    () => ({ ["--json-depth" as string]: line.depth }) as CSSProperties,
    [line.depth],
  );

  return (
    <div className={styles.line} style={style}>
      <div className={styles.lineContent}>{line.segments.map((seg, index) => renderSegment(seg, index))}</div>
    </div>
  );
});

function renderSegment(seg: JsonSegment, index: number): ReactNode {
  switch (seg.kind) {
    case "key":
      return (
        <span className={styles.key} key={index}>
          {seg.text}
        </span>
      );
    case "punct":
      return (
        <span className={styles.punct} key={index}>
          {seg.text}
        </span>
      );
    case "string":
      return (
        <span className={styles.string} key={index}>
          {seg.text}
        </span>
      );
    case "number":
      return (
        <span className={styles.number} key={index}>
          {seg.text}
        </span>
      );
    case "boolean":
      return (
        <span className={styles.boolean} key={index}>
          {seg.text}
        </span>
      );
    case "null":
      return (
        <span className={styles.null} key={index}>
          {seg.text}
        </span>
      );
    case "sensitive":
      return <SensitiveString key={index} path={seg.path} value={seg.value} />;
    default: {
      const _exhaustive: never = seg;
      return _exhaustive;
    }
  }
}

const SensitiveString = memo(function SensitiveString({
  value,
  path,
}: {
  value: string;
  path: string;
}) {
  const { t } = useTranslation();
  const { isExpanded, toggle } = useExpandApi();
  const expanded = isExpanded(path);
  const encoded = useMemo(() => JSON.stringify(value), [value]);
  const label = expanded ? t("jsonCollapse") : t("jsonExpand");

  return (
    <span
      className={`${styles.secret} ${expanded ? styles.secretExpanded : styles.secretCollapsed}`}
    >
      <span className={styles.secretText}>{encoded}</span>
      <span className={styles.secretActions}>
        <CopyIconButton className={styles.iconButton} size={12} text={value} />
        <Tooltip content={label}>
          <button
            aria-expanded={expanded}
            aria-label={label}
            className={styles.toggle}
            onClick={() => toggle(path)}
            type="button"
          >
            {expanded ? (
              <ChevronDown aria-hidden="true" size={14} />
            ) : (
              <ChevronRight aria-hidden="true" size={14} />
            )}
          </button>
        </Tooltip>
      </span>
    </span>
  );
});
