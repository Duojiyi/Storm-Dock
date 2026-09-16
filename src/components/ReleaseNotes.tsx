import type { ReactNode } from "react";
import {
  hasReleaseNotes,
  parseReleaseNotes,
  type BlockNode,
  type InlineNode,
} from "../lib/releaseNotes";
import styles from "./ReleaseNotes.module.css";

function renderInline(nodes: InlineNode[], keyPrefix: string): ReactNode[] {
  return nodes.map((node, index) => {
    const key = `${keyPrefix}-${index}`;
    switch (node.type) {
      case "text":
        return <span key={key}>{node.value}</span>;
      case "bold":
        return <strong key={key}>{renderInline(node.children, key)}</strong>;
      case "code":
        return (
          <code className={styles.code} key={key}>
            {node.value}
          </code>
        );
      case "link":
        return (
          <a
            className={styles.link}
            href={node.href}
            key={key}
            rel="noopener noreferrer"
            target="_blank"
          >
            {renderInline(node.children, key)}
          </a>
        );
      default:
        return null;
    }
  });
}

function renderBlock(block: BlockNode, index: number): ReactNode {
  const key = `b-${index}`;
  switch (block.type) {
    case "heading": {
      const Tag = block.level === 2 ? "h4" : "h5";
      return (
        <Tag className={block.level === 2 ? styles.h2 : styles.h3} key={key}>
          {renderInline(block.children, key)}
        </Tag>
      );
    }
    case "paragraph":
      return (
        <p className={styles.p} key={key}>
          {renderInline(block.children, key)}
        </p>
      );
    case "ul":
      return (
        <ul className={styles.ul} key={key}>
          {block.items.map((item, i) => (
            <li key={`${key}-i-${i}`}>{renderInline(item, `${key}-i-${i}`)}</li>
          ))}
        </ul>
      );
    case "ol":
      return (
        <ol className={styles.ol} key={key}>
          {block.items.map((item, i) => (
            <li key={`${key}-i-${i}`}>{renderInline(item, `${key}-i-${i}`)}</li>
          ))}
        </ol>
      );
    default:
      return null;
  }
}

/** Safe, lightweight renderer for GitHub Release / Keep a Changelog notes. */
export function ReleaseNotes({ markdown }: { markdown?: string | null }) {
  if (!hasReleaseNotes(markdown)) return null;
  const blocks = parseReleaseNotes(markdown!);
  return (
    <div className={styles.root} data-testid="release-notes">
      {blocks.map((block, index) => renderBlock(block, index))}
    </div>
  );
}
