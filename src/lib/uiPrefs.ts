/**
 * Where the view preferences the redesign added survive a restart
 * (`docs/UI_REDESIGN_PLAN.md` §3.7, §6 Phase 6).
 *
 * `get_app_session`/`set_app_session` rather than `localStorage`: these settle
 * where the workspace override and the tray preference already live, so they
 * travel with the profile instead of with the webview's origin, and no schema
 * change is needed to get there. The store is flat `string -> string`, which
 * decides the rest of this module:
 *
 *  - **One key per preference, global.** A flat store can only fake per-project
 *    or per-feature scoping by building keys, and a density the user has to
 *    re-choose on every run is not a preference.
 *  - **One density key for both lists.** `density.ts` records `Density` and
 *    `DEFAULT_DENSITY` as deliberately one decision across the timeline and the
 *    pipeline list; a second key would let the two surfaces disagree about a
 *    choice made once.
 *  - **Every value arrives as an untrusted string**, so a read decodes through
 *    a guard (AGENTS.md §3) and anything unusable — a hand-edited row, a value
 *    an older build wrote — resolves to the in-memory default. A corrupt row
 *    costs a preference, never a view.
 *
 * The pipeline filter persists its segment and its sort, and **not its query**.
 * A restored search string hides rows for a reason the user last saw days ago,
 * so it reads as features having disappeared rather than as a filter still on.
 *
 * Two orderings this module enforces rather than asks callers for:
 *
 *  - **A write is dropped until that preference has been read.** A view mounts
 *    holding the in-memory default and only afterwards learns the stored value,
 *    so a write on first paint overwrites what it is about to load. `read()`
 *    arms `write()` — including when the read failed, since the user's next
 *    choice is still worth storing over a value nothing could parse.
 *  - **Writes debounce on the trailing edge.** A divider drag commits per
 *    release and a toggle can be hammered; only the last value of a burst is
 *    worth a round-trip, and a leading edge would store the first.
 */

import { invoke } from '@tauri-apps/api/core';

import type { RunViewMode } from '../components/RunViewToggle';
import { DEFAULT_DENSITY, isDensity, type Density } from './density';
import {
  DEFAULT_PIPELINE_FILTER,
  type PipelineSegment,
  type PipelineSort,
} from './pipelineFilter';

export const UI_PREF_WRITE_DEBOUNCE_MS = 400;

export interface UiPref<T> {
  /** The `app_session` row this preference occupies. */
  readonly key: string;
  /** The stored value, or the default for a missing, unusable or unreachable one. */
  read(): Promise<T>;
  /** Store `value` after the debounce window, if `read` has resolved. */
  write(value: T): void;
  /**
   * Drop an armed write and disarm the preference.
   *
   * These instances are module singletons, so a pending timer outlives the
   * component that armed it and, in a test run, the test that armed it: a
   * suite driving a persisted control under real timers had its write land
   * inside whichever test happened to be running 400 ms later, where the
   * global `clearAllMocks` made it read as that test's own. `src/test/setup.ts`
   * calls this per test so the leak cannot cross one.
   */
  cancelPendingWrite(): void;
}

export interface UiPrefSpec<T> {
  key: string;
  fallback: T;
  /** `undefined` for anything this build cannot use; the reader gets `fallback`. */
  decode: (raw: string) => T | undefined;
  encode: (value: T) => string;
}

/**
 * Build one preference. The five below are its call sites; it is exported so the
 * arming and debounce rules can be exercised on an instance no earlier test has
 * read, which is the only place either is observable.
 */
export function definePref<T>({ key, fallback, decode, encode }: UiPrefSpec<T>): UiPref<T> {
  let armed = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: { value: T } | null = null;

  const flush = () => {
    timer = null;
    const next = pending;
    pending = null;
    if (next) void commit(key, encode(next.value));
  };

  return {
    key,

    async read(): Promise<T> {
      // A value still inside the debounce window is the newest one there is —
      // the store just does not know it yet. Reading past it is visible on the
      // one key two surfaces share: choose Compact in the project view, open a
      // feature inside 400 ms, and the run timeline mounts on the value the
      // user just replaced, then keeps it for the life of that mount.
      if (pending) {
        armed = true;
        return pending.value;
      }
      try {
        const raw: unknown = await invoke('get_app_session', { key });
        if (typeof raw !== 'string') return fallback;
        const decoded = decode(raw);
        return decoded === undefined ? fallback : decoded;
      } catch {
        return fallback;
      } finally {
        armed = true;
      }
    },

    write(value: T): void {
      if (!armed) return;
      pending = { value };
      if (timer !== null) clearTimeout(timer);
      timer = setTimeout(flush, UI_PREF_WRITE_DEBOUNCE_MS);
    },

    cancelPendingWrite(): void {
      if (timer !== null) clearTimeout(timer);
      timer = null;
      pending = null;
      armed = false;
    },
  };
}

async function commit(key: string, value: string): Promise<void> {
  try {
    await invoke('set_app_session', { key, value });
  } catch {
    /* Dropped, and `errorBus` deliberately not reached from here: a preference
       that failed to store leaves the user nothing to act on, and the surface
       that writes most is a divider drag, which would raise a toast over a
       gesture already finished. The cost is a silently lost preference. */
  }
}

/**
 * Written as exhaustive records so that adding a variant to one of these unions
 * fails to compile here, rather than making that variant silently unpersistable.
 * Membership is a comparison against the stored `true` and not `value in table`,
 * which answers true for `toString` and every other prototype key — and what
 * reaches these is whatever string the store held.
 */
function isMember<T extends string>(table: Record<T, true>, value: string): value is T {
  return table[value as T] === true;
}

const RUN_VIEW_MODES: Record<RunViewMode, true> = { graph: true, timeline: true };

const PIPELINE_SEGMENTS: Record<PipelineSegment, true> = {
  all: true,
  'needs-you': true,
  active: true,
  done: true,
};

const PIPELINE_SORTS: Record<PipelineSort, true> = {
  'needs-you-first': true,
  newest: true,
  oldest: true,
};


export const densityPref: UiPref<Density> = definePref({
  key: 'ui.density',
  fallback: DEFAULT_DENSITY,
  decode: (raw) => (isDensity(raw) ? raw : undefined),
  encode: (value) => value,
});

/**
 * `null` is "never chosen", and it round-trips through the empty string rather
 * than through a number: `FeatureDetail` reads it as "derive the opening width
 * from the measured column", so encoding it as a width would freeze the pane at
 * whatever proportion one window size once produced.
 */
export const inspectorWidthPref: UiPref<number | null> = definePref({
  key: 'ui.inspector_width',
  fallback: null,
  decode: (raw) => {
    const width = Number(raw);
    return Number.isFinite(width) && width > 0 ? width : undefined;
  },
  encode: (value) => (value === null ? '' : String(value)),
});

/** Graph, per UI_REDESIGN_PLAN §7 and `useRunGraph`'s initialiser — a stored
 *  timeline is a choice, and the absence of one is not a vote for it. */
export const runViewModePref: UiPref<RunViewMode> = definePref({
  key: 'ui.run_view_mode',
  fallback: 'graph',
  decode: (raw) => (isMember(RUN_VIEW_MODES, raw) ? raw : undefined),
  encode: (value) => value,
});

export const pipelineSegmentPref: UiPref<PipelineSegment> = definePref({
  key: 'ui.pipeline_segment',
  fallback: DEFAULT_PIPELINE_FILTER.segment,
  decode: (raw) => (isMember(PIPELINE_SEGMENTS, raw) ? raw : undefined),
  encode: (value) => value,
});

export const pipelineSortPref: UiPref<PipelineSort> = definePref({
  key: 'ui.pipeline_sort',
  fallback: DEFAULT_PIPELINE_FILTER.sort,
  decode: (raw) => (isMember(PIPELINE_SORTS, raw) ? raw : undefined),
  encode: (value) => value,
});

/**
 * Every preference this module defines, so a caller that has to reach all of
 * them does not maintain its own list — `src/test/setup.ts` drains their
 * debounced writes between tests, and a preference missing from that drain
 * leaks into whichever test runs 400 ms later.
 */
export const UI_PREFS: readonly UiPref<unknown>[] = [
  densityPref,
  inspectorWidthPref,
  runViewModePref,
  pipelineSegmentPref,
  pipelineSortPref,
] as UiPref<unknown>[];

