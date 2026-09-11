export const COALESCED_REFRESH_MIN_INTERVAL_MS = 500;

export interface CoalescedRefresh {
  /** Request a refresh: at most one in flight, at most one queued. */
  trigger: () => void;
  /**
   * Cancel a queued refresh and drop the queue. Safe to call more than
   * once, and the scheduler stays usable afterwards — StrictMode runs
   * an effect's cleanup between the two mounts of the same fiber, so a
   * disposer that poisoned the instance would leave the rail dead in
   * development only.
   */
  dispose: () => void;
}

export function createCoalescedRefresh(refresh: () => Promise<void>): CoalescedRefresh {
  let running = false;
  let pending = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let lastStartedAt = Number.NEGATIVE_INFINITY;

  const settle = (): void => {
    running = false;
    if (pending) schedule();
  };

  const start = (): void => {
    pending = false;
    running = true;
    lastStartedAt = performance.now();
    // A `refresh` that throws before returning its promise never reaches
    // `.then`, so without this `running` would stay true forever and
    // every later trigger would return early — the rail freezing for the
    // rest of the session with nothing surfaced.
    try {
      refresh().then(settle, settle);
    } catch {
      settle();
    }
  };

  const schedule = (): void => {
    const wait = lastStartedAt + COALESCED_REFRESH_MIN_INTERVAL_MS - performance.now();
    if (wait <= 0) {
      start();
      return;
    }
    timer = setTimeout(() => {
      timer = null;
      start();
    }, wait);
  };

  return {
    trigger: () => {
      pending = true;
      if (running || timer !== null) return;
      schedule();
    },
    dispose: () => {
      pending = false;
      if (timer === null) return;
      clearTimeout(timer);
      timer = null;
    },
  };
}
