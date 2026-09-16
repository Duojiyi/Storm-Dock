const PREF_KEY = "loginBrowser";
const LAST_KEY = "lastLoginBrowser";

export const ASK_BROWSER = "ask";
export const LAST_BROWSER = "last";
export const DEFAULT_BROWSER = "default";

export type LoginBrowser = { id: string; name: string };

export function getLoginBrowserPref(): string {
  const stored = localStorage.getItem(PREF_KEY)?.trim();
  return stored || ASK_BROWSER;
}

export function setLoginBrowserPref(value: string) {
  const next = value.trim() || ASK_BROWSER;
  localStorage.setItem(PREF_KEY, next);
}

export function getLastLoginBrowser(): string | undefined {
  const stored = localStorage.getItem(LAST_KEY)?.trim();
  return stored || undefined;
}

export function rememberLoginBrowser(id: string) {
  const value = id.trim();
  if (!value || value === ASK_BROWSER || value === LAST_BROWSER) return;
  localStorage.setItem(LAST_KEY, value);
}

export function loginNeedsPicker(pref = getLoginBrowserPref(), last = getLastLoginBrowser()) {
  return pref === ASK_BROWSER || (pref === LAST_BROWSER && !last);
}

export function resolveLoginBrowserId(pref = getLoginBrowserPref(), last = getLastLoginBrowser()) {
  if (pref === ASK_BROWSER || pref === LAST_BROWSER) return last ?? DEFAULT_BROWSER;
  return pref;
}
