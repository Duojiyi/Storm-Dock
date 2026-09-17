import { ArrowLeft, ChartNoAxesCombined, FileOutput, LoaderCircle, RefreshCw } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { useTranslation } from "react-i18next";
import { ExportDialog } from "../../components/ExportDialog";
import { Toast, ToastMessage } from "../../components/ToastMessage";
import toastStyles from "../../components/ToastMessage.module.css";
import { Tooltip } from "../../components/Tooltip";
import { WindowDragSurface } from "../../components/WindowDragSurface";
import "../../i18n";
import { listAccounts } from "../../lib/api";
import { applicationKindFromQuery, homePath, syncDocumentAppKind, type Account, type CursorUsageDetails } from "../../lib/types";
import { subscriptionPlanName } from "../home/lib/accountPresentation";
import "../../styles/global.css";
import { daysUntil, hasLimit, isOverLimit, metric, productParts, spendCents } from "./format";
import { EventLedger } from "./EventLedger";
import { ModelBars } from "./ModelBars";
import { WeeklyChart } from "./WeeklyChart";
import styles from "./page.module.css";

function membershipLabel(type?: string, t?: (key: string, options?: Record<string, unknown>) => string) {
  if (!type || !t) return;
  return subscriptionPlanName(type, t);
}

function usageUsedCopy(
  label: string,
  value: CursorUsageDetails["primary"] | NonNullable<CursorUsageDetails["onDemand"]> | NonNullable<CursorUsageDetails["grokBot"]>,
  t: (key: string, options?: Record<string, unknown>) => string,
  parts?: string,
) {
  const amount = metric(value);
  if (parts) return t("usageUsedWithParts", { label, amount, parts });
  if (value.kind === "currency" && hasLimit(value)) {
    return t("usageUsedWithLimit", { label, amount, percent: Math.round(Math.max(value.percent, 0)) });
  }
  if (value.kind === "currency" || value.kind === "percent") {
    return t("usageUsed", { label, amount });
  }
  return `${label} ${amount}`;
}

function resetCopy(resetAt: string | undefined, t: (key: string, options?: Record<string, unknown>) => string) {
  const days = daysUntil(resetAt);
  if (days === undefined) return t("usageUnknown");
  if (days > 0) return t("usageResetsIn", { count: days });
  if (days === 0) return t("usageResetsToday");
  return t("usageResetPassed");
}

function formatResetAt(resetAt: string | undefined) {
  if (!resetAt) return;
  const date = new Date(resetAt);
  if (Number.isNaN(date.getTime())) return;
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(date);
}

function botResetCopy(resetAt: string | undefined, t: (key: string, options?: Record<string, unknown>) => string) {
  const relative = resetCopy(resetAt, t);
  const exact = formatResetAt(resetAt);
  if (!exact || relative === t("usageUnknown")) return relative;
  return t("usageResetsWithTime", { relative, time: exact });
}

function UsagePage() {
  const { t } = useTranslation();
  const search = new URLSearchParams(window.location.search);
  const accountId = search.get("accountId") ?? "";
  const usageKind = search.get("kind") === "grok" ? "grok" : "cursor";
  const isGrok = usageKind === "grok";
  const usageTitle = t(isGrok ? "usageTitleGrok" : "usageTitle");
  const usageLoadingKey = isGrok ? "usageLoadingGrok" : "usageLoading";
  const primaryLabel = t(isGrok ? "usagePrimaryGrok" : "usagePrimary");
  const onDemandLabel = t(isGrok ? "usageOnDemandGrok" : "usageOnDemand");
  const weeklyTitle = t(isGrok ? "usageWeeklyGrok" : "usageWeekly");
  const modelsTitle = t(isGrok ? "usageModelsGrok" : "usageModels");
  const noModelsCopy = t(isGrok ? "usageNoModelsGrok" : "usageNoModels");
  const [data, setData] = useState<CursorUsageDetails>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const [noticeStatus, setNoticeStatus] = useState<"loading" | "success" | "error">("success");
  const [justUpdated, setJustUpdated] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [exportData, setExportData] = useState<unknown>();
  const [accountStatus, setAccountStatus] = useState<Account["status"]>();
  const flashTimer = useRef<number | undefined>(undefined);
  useEffect(() => { syncDocumentAppKind(usageKind); }, [usageKind]);
  useEffect(() => () => window.clearTimeout(flashTimer.current), []);
  useEffect(() => {
    let cancelled = false;
    setData(undefined);
    setError(undefined);
    setAccountStatus(undefined);
    if (!accountId) return () => { cancelled = true; };
    void invoke<CursorUsageDetails | null>(usageKind === "grok" ? "get_saved_grok_usage" : "get_saved_cursor_usage", { id: accountId })
      .then((usage) => {
        if (!cancelled && usage?.accountId === accountId) setData(usage);
      })
      .catch((error) => {
        if (!cancelled) setError(error instanceof Error ? error.message : String(error));
      });
    void listAccounts(usageKind)
      .then((accounts) => {
        if (cancelled) return;
        const match = accounts.find((account) => account.id === accountId);
        setAccountStatus(match?.status);
      })
      .catch(() => {
        /* status badge is optional */
      });
    return () => { cancelled = true; };
  }, [accountId, usageKind]);
  const refresh = async () => {
    if (!accountId || busy) { if (!accountId) setError(t("usageUnknown")); return; }
    setBusy(true);
    setError(undefined);
    setNoticeStatus("loading");
    setNotice(t(usageLoadingKey));
    try {
      const usage = await invoke<CursorUsageDetails>(usageKind === "grok" ? "get_grok_usage" : "get_cursor_usage", { id: accountId });
      if (usage.accountId !== accountId) throw new Error(t("usageAccountMismatch"));
      setData(usage);
      setAccountStatus(undefined);
      void listAccounts(usageKind).then((accounts) => {
        const match = accounts.find((account) => account.id === accountId);
        setAccountStatus(match?.status);
      }).catch(() => undefined);
      setNoticeStatus("success");
      setNotice(t("usageRefreshed"));
      setJustUpdated(true);
      window.clearTimeout(flashTimer.current);
      flashTimer.current = window.setTimeout(() => setJustUpdated(false), 1400);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const lower = message.toLowerCase();
      if (lower.includes("user account is blocked") || message.includes("账号已被封禁") || message.includes("账号已封禁")) {
        setAccountStatus("blocked");
      } else if (message.includes("失效") || message.includes("过期")) {
        setAccountStatus((current) => current === "blocked" ? current : "invalid");
      }
      setError(message);
      setNoticeStatus("error");
      setNotice(message);
    } finally {
      setBusy(false);
    }
  };
  const openExport = async () => {
    if (!accountId) return;
    try { setExportData(await invoke<unknown>(usageKind === "grok" ? "get_grok_bot_export_record" : "get_cursor_export_record", { id: accountId })); setExportOpen(true); }
    catch (error) { setError(error instanceof Error ? error.message : String(error)); }
  };
  const membership = membershipLabel(data?.membershipType, t);
  const resetExact = formatResetAt(data?.resetAt);
  const checkedAt = data ? new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(new Date(data.checkedAt * 1000)) : undefined;
  return <Toast.Provider>
    <main className={styles.shell}>
    <WindowDragSurface />
    <header className={styles.header}>
      <a aria-label={t("back")} className={styles.back} href={homePath(usageKind)}><ArrowLeft aria-hidden="true" size={20} /></a>
      <h1>{usageTitle}</h1>
      <div className={styles.actions}>
        <Tooltip content={t("export")}><button aria-label={t("export")} className={styles.export} disabled={busy} onClick={() => void openExport()} type="button"><FileOutput aria-hidden="true" size={18} /></button></Tooltip>
        <button aria-busy={busy} aria-label={busy ? t(usageLoadingKey) : t("usageRefresh")} className={`${styles.refresh} ${busy ? styles.refreshBusy : ""}`} onClick={() => void refresh()} type="button">
          {busy ? <LoaderCircle aria-hidden="true" className={styles.spinning} size={17} /> : <RefreshCw aria-hidden="true" size={17} />}
          {t("usageRefresh")}
        </button>
      </div>
    </header>
    {error && !(notice && noticeStatus === "error") && <p className={styles.error}>{error}</p>}
    {!data && accountStatus === "blocked" && <p className={styles.blockedBanner}>{t("usageAccountBlocked", { defaultValue: "此账号已被封禁，无法刷新用量。请更换账号或联系服务方。" })}</p>}
    {!data && !busy && !error && <section className={styles.empty}><ChartNoAxesCombined aria-hidden="true" size={48} /><h2>{t("usageEmptyTitle")}</h2><p>{t("usageEmptyDescription")}</p></section>}
    {!data && busy && <section className={styles.empty}><LoaderCircle aria-hidden="true" className={styles.spinning} size={28} /><h2>{t(usageLoadingKey)}</h2></section>}
    {data && <section className={styles.workspace}>
      {busy && <div aria-hidden="true" className={styles.indeterminate}><span /></div>}
      <div className={styles.identity}>
        <strong>{data.email ?? data.label}</strong>
        {membership && <span className={styles.badge}>{membership}</span>}
        {accountStatus === "blocked" && <span className={styles.blockedBadge}>{t("tokenBlocked", { defaultValue: "账号已封禁" })}</span>}
        {accountStatus === "invalid" && <span className={styles.invalidBadge}>{t("tokenInvalid", { defaultValue: "Token已失效" })}</span>}
        {accountStatus === "missing" && <span className={styles.invalidBadge}>{t("credentialMissing", { defaultValue: "凭证缺失" })}</span>}
      </div>
      {accountStatus === "blocked" && <p className={styles.blockedBanner}>{t("usageAccountBlocked", { defaultValue: "此账号已被封禁，无法刷新用量。请更换账号或联系服务方。" })}</p>}
      <div className={styles.usageLines}>
        <p className={isOverLimit(data.primary) ? `${styles.usageLine} ${styles.overLimit}` : styles.usageLine}>{usageUsedCopy(primaryLabel, data.primary, t, isGrok ? productParts(data.products) : undefined)}{isGrok && resetExact ? <span className={styles.poolReset}>{t("usageResetParen", { reset: botResetCopy(data.resetAt, t) })}</span> : null}</p>
        {data.onDemand && <p className={isOverLimit(data.onDemand) ? `${styles.usageLine} ${styles.overLimit}` : styles.usageLine}>{usageUsedCopy(onDemandLabel, data.onDemand, t)}</p>}
        {data.grokBot && <p className={styles.usageLine}>{usageUsedCopy(t("usageGrokBot"), data.grokBot, t)}{data.grokBotResetAt && <span className={styles.poolReset} title={formatResetAt(data.grokBotResetAt)}> · {botResetCopy(data.grokBotResetAt, t)}</span>}</p>}
      </div>
      <div className={styles.charts}>
        <section><h2>{weeklyTitle}</h2>{data.weeklyAvailable ? <WeeklyChart days={data.weekly} events={data.events ?? []} /> : <p className={styles.muted}>{data.weeklyError ?? t("usageWeeklyUnavailable")}</p>}</section>
        <section><h2>{modelsTitle}</h2>{(data.events ?? []).some((event) => spendCents(event) !== undefined) ? <ModelBars events={data.events ?? []} models={data.models} /> : <p className={styles.muted}>{noModelsCopy}</p>}</section>
      </div>
      <EventLedger events={data.events ?? []} unavailable={data.weeklyError} />
      <div className={styles.meta}>
        {resetExact ? <Tooltip content={resetExact}><button className={styles.reset} type="button">{resetCopy(data.resetAt, t)}</button></Tooltip> : <span>{t("usageUnknown")}</span>}
        <span className={justUpdated ? styles.justUpdated : undefined}>{t("usageCheckedAt", { time: checkedAt })}</span>
      </div>
    </section>}
    {exportData !== undefined && <ExportDialog data={[exportData]} filename={`${usageKind}-account-${accountId}.json`} onOpenChange={setExportOpen} open={exportOpen} />}
  </main>
  <ToastMessage notice={notice} onOpenChange={(open) => { if (!open) setNotice(undefined); }} status={noticeStatus} />
  <Toast.Viewport className={toastStyles.viewport} />
  
  </Toast.Provider>;
}

const bootKind = applicationKindFromQuery();
syncDocumentAppKind(bootKind);
createRoot(document.getElementById("root")!).render(<UsagePage />);
