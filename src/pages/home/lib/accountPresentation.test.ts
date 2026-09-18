import { describe, expect, it } from "vitest";
import type { Account } from "../../../lib/types";
import { subscriptionLabel, subscriptionPlanBadge, usageLabel, endpointHost, accountKindKey, grokBotUsageLabel, isGrokBotListEligible } from "./accountPresentation";

const t = (key: string, options?: Record<string, unknown>) =>
  `${key}:${options?.count ?? options?.amount ?? options?.percent ?? ""}`;
const account: Account = { id: "a", label: "A", importType: "oauth", subscription: { plan: "Pro", expiresAt: 1 }, daysRemaining: 2, isCurrent: false };

describe("account presentation", () => {
  it("does not default unqueried plans to Free", () => {
    expect(subscriptionPlanBadge({ ...account, application: "grok", subscription: {} }, t)).toEqual({
      name: "subscriptionUnknownPlan:",
      plan: "unknown",
    });
    expect(subscriptionLabel({ ...account, application: "cursor", subscription: {} }, t)).toEqual({
      name: "subscriptionUnknownPlan:",
      expiry: "subscriptionUnknownExpiry:",
      plan: "unknown",
    });
    expect(subscriptionPlanBadge({ ...account, application: "grok", subscription: { plan: "free" } }, t)).toEqual({
      name: "subscriptionPlans.free:",
      plan: "free",
    });
  });

  it("formats grok subscription days with the same 24h floor as grok bot", () => {
    const end = new Date(Date.now() + 6 * 86_400_000 + 3_600_000);
    expect(subscriptionLabel({
      ...account,
      application: "grok",
      subscription: {
        plan: "supergrok_heavy",
        billingCycleEnd: end.toISOString(),
        expiresAt: Math.floor(end.getTime() / 1000),
      },
      daysRemaining: 5,
    }, t).expiry).toBe("subscriptionDays:6");
  });

  it("does not treat grok quota reset as subscription expiry", () => {
    const end = new Date(Date.now() + 2 * 86_400_000 + 3_600_000);
    expect(subscriptionLabel({
      ...account,
      application: "grok",
      subscription: { plan: "supergrok_heavy" },
      resetAt: end.toISOString(),
      daysRemaining: undefined,
    }, t).expiry).toBe("subscriptionUnknownExpiry:");
  });

  it("formats subscription status from account data", () => {
    expect(subscriptionLabel(account, t)).toEqual({ name: "subscriptionPlans.pro:", expiry: "subscriptionDays:2", plan: "pro" });
    expect(subscriptionLabel({ ...account, daysRemaining: -1 }, t)?.expiry).toBe("subscriptionExpired:");
  });

  it("formats currency usage and free accounts", () => {
    expect(usageLabel({ ...account, usage: { kind: "currency", used: 123, percent: 1 } }, t)).toBe("usageSpent:$1.23");
    expect(usageLabel({ ...account, subscription: { plan: "Free" } }, t)).toBe("usageFree:");
  });

  it("formats percent usage and endpoint hosts", () => {
    expect(usageLabel({ ...account, usage: { kind: "percent", used: 12, percent: 12 } }, t)).toBe("usagePercent:12");
    expect(endpointHost("https://api.example.com/v1")).toBe("api.example.com");
  });

  it("appends grok quota reset days after usage", () => {
    const inThreeDays = new Date(Date.now() + 3 * 86_400_000 + 3_600_000).toISOString();
    expect(usageLabel({
      ...account,
      application: "grok",
      usage: { kind: "percent", used: 45, percent: 45 },
      resetAt: inThreeDays,
    }, t)).toBe("usagePercent:45 · grokBotResetDays:3");
  });

  it("maps ChatGPT import types to sign-in vs API Key", () => {
    expect(accountKindKey(account)).toBe("accountKind.account");
    expect(accountKindKey({ ...account, importType: "api_key" })).toBe("accountKind.apiKey");
  });

  it("formats grok bot usage badge with abbreviated reset", () => {
    const inThreeDays = new Date(Date.now() + 3 * 86_400_000).toISOString();
    expect(grokBotUsageLabel({ ...account, grokBotUsage: { kind: "percent", used: 45, percent: 45 }, grokBotResetAt: inThreeDays }, t)).toBe("grokBotUsageBadge:45");
    expect(grokBotUsageLabel({ ...account, grokBotUsage: { kind: "percent", used: 10, percent: 10 } }, t)).toBe("grokBotUsagePercent:10");
  });

  it("excludes unknown subscription, free plan, expired token, and banned accounts from grok bot lists", () => {
    expect(isGrokBotListEligible(account)).toBe(true);
    expect(isGrokBotListEligible({ ...account, subscription: {} })).toBe(false);
    expect(isGrokBotListEligible({ ...account, subscription: { plan: "free" } })).toBe(false);
    expect(isGrokBotListEligible({ ...account, subscription: { plan: "Free" } })).toBe(false);
    expect(isGrokBotListEligible({ ...account, status: "invalid" })).toBe(false);
    expect(isGrokBotListEligible({ ...account, status: "blocked" })).toBe(false);
    expect(isGrokBotListEligible({ ...account, status: "missing" })).toBe(true);
  });
});

  it("formats reset-passed grok usage as 0% · reset", () => {
    const yesterday = new Date(Date.now() - 86_400_000).toISOString();
    expect(usageLabel({
      ...account,
      application: "grok",
      usage: { kind: "percent", used: 100, percent: 100 },
      resetAt: yesterday,
    }, t)).toBe("usagePercent:0 · grokBotResetPassed:");
  });

  it("formats reset-passed grok bot badge as 0% · reset", () => {
    const yesterday = new Date(Date.now() - 86_400_000).toISOString();
    expect(grokBotUsageLabel({
      ...account,
      grokBotUsage: { kind: "percent", used: 100, percent: 100 },
      grokBotResetAt: yesterday,
    }, t)).toBe("grokBotUsageBadge:0");
  });

