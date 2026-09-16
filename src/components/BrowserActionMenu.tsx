import * as AlertDialog from "@radix-ui/react-alert-dialog";
import { invoke } from "@tauri-apps/api/core";
import { Globe } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode, type Ref } from "react";
import { useTranslation } from "react-i18next";
import arcIcon from "../assets/browsers/arc.png";
import braveIcon from "../assets/browsers/brave.png";
import chromeIcon from "../assets/browsers/chrome.png";
import edgeIcon from "../assets/browsers/edge.png";
import firefoxIcon from "../assets/browsers/firefox.png";
import safariIcon from "../assets/browsers/safari.png";
import { DEFAULT_BROWSER as DEFAULT_BROWSER_ID, type LoginBrowser } from "../lib/loginBrowser";
import styles from "./BrowserActionMenu.module.css";

export type { LoginBrowser };

const DEFAULT_BROWSER: LoginBrowser = { id: DEFAULT_BROWSER_ID, name: "System default" };

const BROWSER_ICONS: Record<string, string> = {
  safari: safariIcon,
  chrome: chromeIcon,
  edge: edgeIcon,
  firefox: firefoxIcon,
  brave: braveIcon,
  arc: arcIcon,
};

type BrowserActionMenuProps = {
  label: string;
  disabled?: boolean;
  className?: string;
  children?: ReactNode;
  onSelect: (browserId: string) => void;
};

export function BrowserActionMenu({
  label,
  disabled,
  className,
  children,
  onSelect,
}: BrowserActionMenuProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [browsers, setBrowsers] = useState<LoginBrowser[]>([DEFAULT_BROWSER]);
  const firstRef = useRef<HTMLButtonElement>(null);
  const [preferred, ...installed] = browsers;

  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);

  useEffect(() => {
    if (!open) return;
    void invoke<LoginBrowser[]>("list_login_browsers")
      .then((items) => {
        if (items.length > 0) setBrowsers(items);
      })
      .catch(() => {});
  }, [open]);

  const closeThenSelect = (id: string) => {
    setOpen(false);
    queueMicrotask(() => onSelect(id));
  };

  return (
    <AlertDialog.Root onOpenChange={setOpen} open={open}>
      <AlertDialog.Trigger asChild>
        <button className={className} disabled={disabled} type="button">
          {children}
          {label}
        </button>
      </AlertDialog.Trigger>
      <AlertDialog.Portal>
        <AlertDialog.Overlay className={styles.overlay} />
        <AlertDialog.Content
          className={styles.content}
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            firstRef.current?.focus();
          }}
        >
          <AlertDialog.Title>{t("startLoginChooseBrowser")}</AlertDialog.Title>
          <AlertDialog.Description>{t("chooseLoginBrowserDescription")}</AlertDialog.Description>
          <ul className={styles.list}>
            {preferred ? (
              <li>
                <BrowserRow
                  browser={preferred}
                  buttonRef={firstRef}
                  disabled={disabled}
                  onSelect={closeThenSelect}
                />
              </li>
            ) : null}
            {installed.length > 0 ? <li className={styles.divider} aria-hidden="true" /> : null}
            {installed.map((browser) => (
              <li key={browser.id}>
                <BrowserRow
                  browser={browser}
                  disabled={disabled}
                  onSelect={closeThenSelect}
                />
              </li>
            ))}
          </ul>
          <div className={styles.actions}>
            <AlertDialog.Cancel asChild>
              <button className={styles.cancel} type="button">{t("cancel")}</button>
            </AlertDialog.Cancel>
          </div>
        </AlertDialog.Content>
      </AlertDialog.Portal>
    </AlertDialog.Root>
  );
}

function BrowserRow({
  browser,
  buttonRef,
  disabled,
  onSelect,
}: {
  browser: LoginBrowser;
  buttonRef?: Ref<HTMLButtonElement>;
  disabled?: boolean;
  onSelect: (browserId: string) => void;
}) {
  const { t } = useTranslation();
  const isDefault = browser.id === "default";
  const icon = BROWSER_ICONS[browser.id];
  return (
    <button
      className={styles.row}
      disabled={disabled}
      onClick={() => onSelect(browser.id)}
      ref={buttonRef}
      type="button"
    >
      <span className={`${styles.icon}${browser.id === "arc" ? ` ${styles.arc}` : ""}`} aria-hidden="true">
        {icon ? <img alt="" src={icon} /> : <Globe size={16} strokeWidth={2.2} />}
      </span>
      <span className={styles.copy}>
        <span className={styles.name}>{isDefault ? t("systemDefaultBrowser") : browser.name}</span>
        {isDefault ? <span className={styles.hint}>{t("systemDefaultBrowserHint")}</span> : null}
      </span>
    </button>
  );
}
