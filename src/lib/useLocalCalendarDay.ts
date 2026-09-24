import { useEffect, useState } from "react";
import { localDateKey } from "./calendar";

/**
 * Returns the current local calendar day key and refreshes at local midnight
 * (with a 60s backup tick) so relative "today/tomorrow" labels stay correct
 * while the window stays open overnight.
 */
export function useLocalCalendarDay() {
  const [dayKey, setDayKey] = useState(() => localDateKey());

  useEffect(() => {
    const sync = () => {
      const next = localDateKey();
      setDayKey((current) => (current === next ? current : next));
    };

    const now = new Date();
    const nextMidnight = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
    const untilMidnight = Math.max(1_000, nextMidnight.getTime() - now.getTime());
    const midnightTimer = window.setTimeout(sync, untilMidnight);
    const interval = window.setInterval(sync, 60_000);
    return () => {
      window.clearTimeout(midnightTimer);
      window.clearInterval(interval);
    };
  }, [dayKey]);

  return dayKey;
}
