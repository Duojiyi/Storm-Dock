import type { Account } from "../../../lib/types";

export type Translate = (key: string, options?: Record<string, unknown>) => string;

export function subscriptionPlanBadge(account: Account, t: Translate) {
  const raw = account.subscription.plan?.toLowerCase();
  // Grok Build API returns null tier for Free; avoid "未知订阅" before/without refresh.
  const plan = raw || (account.application === "grok" ? "free" : undefined);
  if (!plan) return { name: t("subscriptionUnknownPlan"), plan: "unknown" };
  return {
    name: t(`subscriptionPlans.${plan}`, { defaultValue: account.subscription.plan ?? plan }),
    plan,
  };
}

export function subscriptionLabel(account: Account, t: Translate) {
  const plan = account.subscription.plan;
  if (!plan) return undefined;
  const normalizedPlan = plan.toLowerCase();
  const name = t(`subscriptionPlans.${normalizedPlan}`, { defaultValue: plan });
  const days = account.daysRemaining;
  const expiry = !account.subscription.expiresAt || days === undefined
    ? t("subscriptionUnknownExpiry")
    : days > 0
      ? t("subscriptionDays", { count: days })
      : days === 0
        ? t("subscriptionToday")
        : t("subscriptionExpired");
  return { name, expiry, plan: normalizedPlan };
}

export function usageLabel(account: Account, t: Translate) {
  if (account.usage?.kind === "currency")
    return t("usageSpent", { amount: `$${(account.usage.used / 100).toFixed(2)}` });
  if (account.usage?.kind === "percent")
    return t("usagePercent", { percent: Math.round(account.usage.percent) });
  return account.subscription.plan?.toLowerCase() === "free" ? t("usageFree") : undefined;
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
  if (isGrokBotFreePlan(account)) return false;
  if (account.application && account.application !== "cursor" && account.application !== "grok") {
    return false;
  }
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
