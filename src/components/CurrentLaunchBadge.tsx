import { Check, Play } from "lucide-react";
import { useTranslation } from "react-i18next";
import styles from "./CurrentLaunchBadge.module.css";

type Props = {
  busy?: boolean;
  onLaunch?: () => void;
};

export function CurrentLaunchBadge({ busy, onLaunch }: Props) {
  const { t } = useTranslation();
  const idle = (
    <span className={`${styles.layer} ${styles.idle}`}>
      <Check aria-hidden="true" size={16} />
      {t("current")}
    </span>
  );
  if (!onLaunch) {
    return <span className={styles.badge}>{idle}</span>;
  }
  return (
    <button
      aria-label={t("launchApp")}
      className={styles.badge}
      disabled={busy}
      onClick={onLaunch}
      type="button"
    >
      {idle}
      <span className={`${styles.layer} ${styles.launch}`}>
        <Play aria-hidden="true" size={16} />
        {t("launchApp")}
      </span>
    </button>
  );
}
