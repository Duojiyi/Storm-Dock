import { afterEach, describe, expect, it } from "vitest";
import {
  ASK_BROWSER,
  DEFAULT_BROWSER,
  LAST_BROWSER,
  getLastLoginBrowser,
  getLoginBrowserPref,
  loginNeedsPicker,
  rememberLoginBrowser,
  resolveLoginBrowserId,
  setLoginBrowserPref,
} from "./loginBrowser";

const memory = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
  value: {
    getItem: (key: string) => memory.get(key) ?? null,
    setItem: (key: string, value: string) => {
      memory.set(key, value);
    },
    removeItem: (key: string) => {
      memory.delete(key);
    },
  },
});

describe("loginBrowser", () => {
  afterEach(() => {
    memory.clear();
  });

  it("defaults to asking every time", () => {
    expect(getLoginBrowserPref()).toBe(ASK_BROWSER);
    expect(loginNeedsPicker()).toBe(true);
    expect(resolveLoginBrowserId()).toBe(DEFAULT_BROWSER);
  });

  it("asks until last-used exists", () => {
    setLoginBrowserPref(LAST_BROWSER);
    expect(loginNeedsPicker()).toBe(true);
    rememberLoginBrowser("chrome");
    expect(getLastLoginBrowser()).toBe("chrome");
    expect(loginNeedsPicker()).toBe(false);
    expect(resolveLoginBrowserId()).toBe("chrome");
  });

  it("uses a pinned browser without asking", () => {
    setLoginBrowserPref("safari");
    expect(loginNeedsPicker()).toBe(false);
    expect(resolveLoginBrowserId()).toBe("safari");
  });

  it("ignores ask/last when remembering a pick", () => {
    rememberLoginBrowser(ASK_BROWSER);
    rememberLoginBrowser(LAST_BROWSER);
    rememberLoginBrowser("  ");
    expect(getLastLoginBrowser()).toBeUndefined();
  });
});
