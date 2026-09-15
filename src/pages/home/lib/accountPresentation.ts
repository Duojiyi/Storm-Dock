import type { Account } from "../../../lib/types";

export type Translate = (key: string, options?: Record<string, unknown>) => string;

export function subscriptionPlanName(plan: string, t: Translate) {
  const normalized = plan.toLowerCase().replace(/[\s-]+/g, "_");
  return t(`subscriptionPlans.${normalized}`, { defaultValue: plan });
}

export function subscriptionPlanBadge(account: Account, t: Translate) {
  const plan = account.subscription.plan?.toLowerCase();
  if (!plan) return { name: t("subscriptionUnknownPlan"), plan: "unknown" };
  return {
    name: subscriptionPlanName(plan, t),
    plan,
  };
}

export function subscriptionLabel(account: Account, t: Translate) {
  const plan = account.subscription.plan;
  if (!plan) {
    return {
      name: t("subscriptionUnknownPlan"),
      expiry: t("subscriptionUnknownExpiry"),
      plan: "unknown",
    };
  }
  const normalizedPlan = plan.toLowerCase();
  const name = subscriptionPlanName(plan, t);
  const days = account.application === "grok"
    ? grokRemainingDays(account.subscription.billingCycleEnd, account.subscription.expiresAt)
      ?? account.daysRemaining
    : account.daysRemaining;
  const expiry = days === undefined
    ? t("subscriptionUnknownExpiry")
    : days > 0
      ? t("subscriptionDays", { count: days })
      : days === 0
        ? t("subscriptionToday")
        : t("subscriptionExpired");
  return { name, expiry, plan: normalizedPlan };
}

function grokRemainingDays(iso: string | undefined, unix?: number) {
  const stamp = iso
    ? new Date(iso).getTime()
    : unix != null
      ? unix * 1000
      : Number.NaN;
  if (Number.isNaN(stamp)) return undefined;
  const days = Math.floor((stamp - Date.now()) / 86_400_000);
  return days < 0 ? -1 : days;
}

export function usageLabel(account: Account, t: Translate) {
  let label: string | undefined;
  if (account.usage?.kind === "currency")
    label = t("usageSpent", { amount: `$${(account.usage.used / 100).toFixed(2)}` });
  else if (account.usage?.kind === "percent")
    label = t("usagePercent", { percent: Math.round(account.usage.percent) });
  else if (account.subscription.plan?.toLowerCase() === "free")
    label = t("usageFree");
  const reset = account.application === "grok" ? grokBotResetLabel(account.resetAt, t) : undefined;
  return label && reset ? `${label} · ${reset}` : label;
}

export function endpointHost(url?: string) {
  if (!url) return undefined;
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

export function accountKindKey(account: Account) {
  return account.importType === "api_key" ? "accountKind.apiKey" : "accountKind.account";
}

export function grokBotResetLabel(resetAt: string | undefined, t: Translate) {
  if (!resetAt) return undefined;
  const reset = new Date(resetAt).getTime();
  if (Number.isNaN(reset)) return undefined;
  const days = Math.floor((reset - Date.now()) / 86_400_000);
  if (days > 0) return t("grokBotResetDays", { count: days });
  if (days === 0) return t("grokBotResetToday");
  return t("grokBotResetPassed");
}


export function isGrokBotFreePlan(account: Account) {
  return account.subscription.plan?.toLowerCase() === "free";
}

/** Cursor paid + any Grok Build account may launch Grok Bot. */
export function canLaunchGrokBot(account: Account) {
  if (account.application === "grok") return true;
  // After the grok early-return, application is narrowed to cursor | codex | undefined.
  if (account.application && account.application !== "cursor") return false;
  if (isGrokBotFreePlan(account)) return false;
  return true;
}

export function grokBotSourceKey(account: Account) {
  return account.application === "grok" ? "grok" : "cursor";
}

export function grokBotUsageLabel(account: Account, t: Translate) {
  const usage = account.grokBotUsage;
  if (!usage) return undefined;
  const percent = Math.round(usage.percent);
  const reset = grokBotResetLabel(account.grokBotResetAt, t);
  return reset ? t("grokBotUsageBadge", { percent, reset }) : t("grokBotUsagePercent", { percent });
}
