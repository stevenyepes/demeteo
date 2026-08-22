import { useEffect, useState } from 'react';

/**
 * Milliseconds since `startedAt`, re-read once a second.
 *
 * The tick is state in whichever component calls this, so it must be called at
 * the leaf that renders the number: a live clock one level up re-renders that
 * level's whole subtree every second for the sake of one string.
 *
 * A `startedAt` of `0` means nothing has started and the hook stays still.
 */
export function useElapsed(startedAt: number, intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (startedAt === 0) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(timer);
  }, [startedAt, intervalMs]);

  return startedAt === 0 ? 0 : Math.max(0, now - startedAt);
}
