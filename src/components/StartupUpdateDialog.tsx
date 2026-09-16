import * as Toast from "@radix-ui/react-toast";
import { Info, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { checkForAppUpdate } from "../lib/updater";
import {
  STARTUP_UPDATE_DELAY_MS,
  readDismissedStartupUpdateVersion,
  rememberDismissedStartupUpdate,
  shouldPromptStartupUpdate,
  shouldRunStartupUpdateCheck
} from "../lib/startupUpdate";
import toastStyles from "./ToastMessage.module.css";

/** Same SPA navigation pattern as the home settings gear (`/settings.html?kind=…`). */
export function settingsAboutPath(search = window.location.search) {
  const params = new URLSearchParams();
  const kind = new URLSearchParams(search).get("kind");
  if (kind) params.set("kind", kind);
  params.set("tab", "about");
  return `/settings.html?${params}`;
}

export function StartupUpdateDialog() {
  const { t } = useTranslation();
  const [update, setUpdate] = useState<{ version: string }>();
  const [open, setOpen] = useState(false);
  const skipRemember = useRef(false);

  useEffect(() => {
    if (!shouldRunStartupUpdateCheck()) return;

    let cancelled = false;
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const result = await checkForAppUpdate();
          if (cancelled) return;
          if (
            !shouldPromptStartupUpdate({
              result,
              dismissedVersion: readDismissedStartupUpdateVersion()
            })
          ) {
            return;
          }
          if (result.status !== "available") return;
          setUpdate({ version: result.version });
          setOpen(true);
        } catch {
          /* ignore check failures silently */
        }
      })();
    }, STARTUP_UPDATE_DELAY_MS);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, []);

  if (!update) return null;

  return (
    <Toast.Root
      className={`${toastStyles.toast} ${toastStyles.info}`}
      duration={Infinity}
      onOpenChange={(next) => {
        if (next) {
          setOpen(true);
          return;
        }
        if (!skipRemember.current) {
          rememberDismissedStartupUpdate(update.version);
        }
        skipRemember.current = false;
        setOpen(false);
      }}
      open={open}
    >
      <Info aria-hidden="true" size={17} />
      <Toast.Description>{t("startupUpdateTitle")}</Toast.Description>
      <Toast.Action
        altText={t("startupUpdateGo")}
        className={toastStyles.action}
        onClick={() => {
          skipRemember.current = true;
          setOpen(false);
          window.location.assign(settingsAboutPath());
        }}
      >
        {t("startupUpdateGo")}
      </Toast.Action>
      <Toast.Close aria-label={t("startupUpdateLater")} className={toastStyles.close}>
        <X aria-hidden="true" size={14} strokeWidth={2} />
      </Toast.Close>
    </Toast.Root>
  );
}
