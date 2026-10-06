export type DetachedCap = { value: number | undefined; error: string | null };

/** Blank is "no cap". Anything else must be a value `launch_run` accepts —
 *  a non-positive cap is refused there, and is never silently dropped here
 *  the way `maxBudgetUsd` is, because that would launch an uncapped paid run
 *  the user meant to bound. Wall-clock is whole minutes: it reaches the
 *  runner as `max_wall_clock_secs: u64`. The cap inputs are `type="text"`
 *  for this: a number input reports unparseable text as `''`, which would
 *  read here as "no cap". */
export function parseDetachedCap(raw: string, kind: 'cost' | 'wallClock'): DetachedCap {
  const text = raw.trim();
  if (text === '') return { value: undefined, error: null };
  const n = Number(text);
  if (kind === 'cost') {
    return Number.isFinite(n) && n > 0
      ? { value: n, error: null }
      : { value: undefined, error: 'Enter an amount greater than 0, or leave blank for no cap.' };
  }
  return Number.isInteger(n) && n > 0
    ? { value: n, error: null }
    : { value: undefined, error: 'Enter a whole number of minutes above 0, or leave blank for no cap.' };
}

export const NO_CAP: DetachedCap = { value: undefined, error: null };
