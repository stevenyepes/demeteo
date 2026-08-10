import { useCallback, useEffect, useState } from 'react';

import type { UiPref } from '../lib/uiPrefs';

/**
 * One `UiPref` bound to component state: read once on mount, written on every
 * choice the user makes (`docs/UI_REDESIGN_PLAN.md` §6 Phase 6).
 *
 * `initial` is what the view holds until that read answers, and it has to be
 * the value the preference itself falls back to — the two are one default
 * spelled twice, and a mount that finds nothing stored visibly flips if they
 * disagree. It is a parameter rather than something read off `pref` because
 * `useState` cannot be seeded from a store reached over IPC.
 *
 * **The write belongs in the setter, not in an effect on `value`.** An effect
 * cannot tell a user's choice from the read's own answer, so every mount would
 * store back what it just restored: an IPC round-trip per view opened, and a
 * race against a choice made while the read was still in flight. `uiPrefs`
 * drops the write that precedes the first read and would take every one after
 * it, so the mistake survives its own first test.
 */
export function usePersistedPref<T>(pref: UiPref<T>, initial: T): [T, (next: T) => void] {
  const [value, setValue] = useState<T>(initial);

  useEffect(() => {
    let cancelled = false;
    void pref.read().then((stored) => {
      if (!cancelled) setValue(stored);
    });
    return () => {
      cancelled = true;
    };
  }, [pref]);

  const set = useCallback(
    (next: T) => {
      setValue(next);
      pref.write(next);
    },
    [pref],
  );

  return [value, set];
}
