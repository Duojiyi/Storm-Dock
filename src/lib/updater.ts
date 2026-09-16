import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";

export type UpdateCheckResult =
  | { status: "up-to-date" }
  | { status: "available"; version: string; notes?: string; date?: string };

export async function getCurrentVersion(): Promise<string> {
  try {
    return await getVersion();
  } catch {
    return "";
  }
}

export async function checkForAppUpdate(): Promise<UpdateCheckResult> {
  const currentVersion = await getCurrentVersion();
  const { check } = await import("@tauri-apps/plugin-updater");
  try {
    const update = await check({ timeout: 30_000 });
    if (!update) {
      console.info("[updater] up-to-date", { currentVersion });
      return { status: "up-to-date" };
    }
    console.info("[updater] available", {
      currentVersion,
      version: update.version,
      date: update.date
    });
    return {
      status: "available",
      version: update.version,
      notes: update.body ?? undefined,
      date: update.date ?? undefined
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    console.error("[updater] check failed", { currentVersion, message, error });
    throw error instanceof Error ? error : new Error(message);
  }
}

/** Download, install, and restart via the Rust command (safer on macOS). */
export function installUpdateAndRestart(): Promise<boolean> {
  return invoke<boolean>("install_update_and_restart");
}
