/**
 * The one sentence a staleness chip is allowed to say, decided here so it can
 * be asserted without rendering anything.
 *
 * Three states, never two. "Up to date" and "we could not measure it" look the
 * same from a caller that reads a missing count as zero, and that reading is
 * the reason nothing in Demeteo could say a branch was behind until now: a
 * number nobody could take rendered as a branch nobody needs to sync.
 */

import type { BranchDivergence, FeatureDrift } from '../types';
import type { RunStatusTone } from './runStatus';

export interface StalenessChip {
  label: string;
  tone: RunStatusTone;
  /** Hover text: the count on its own does not say what it was counted
   *  against, nor how old the ref it was counted against is. */
  title: string;
}

export function describeStaleness(drift: FeatureDrift | null): StalenessChip | null {
  if (drift === null) return null;

  const { behind } = drift.divergence;
  // A reading that failed before it resolved a base still has to render, and
  // naming no ref is better than naming the wrong one.
  const base = drift.base_ref || 'the base branch';
  const asOf = drift.fetched
    ? 'as of just now'
    : `as of the last time ${base} was fetched`;

  if (behind === null) {
    return {
      label: 'Drift unknown',
      tone: 'slate',
      title: `Demeteo could not count this branch against ${base}. That is not the same as being up to date.`,
    };
  }
  if (behind === 0) {
    return {
      label: 'Up to date',
      tone: 'emerald',
      title: `Nothing on ${base} is missing from this branch, ${asOf}.`,
    };
  }
  return {
    label: behind === 1 ? '1 behind' : `${behind} behind`,
    tone: 'cyan',
    title: `${behind} commit${behind === 1 ? '' : 's'} on ${base} are not on this branch, ${asOf}. Sync to merge them in.`,
  };
}

/** Whether a drift reading is worth interrupting the user about — the one
 *  question a caller that only wants to decorate should ask. */
export function isBehind(divergence: BranchDivergence): boolean {
  return divergence.behind !== null && divergence.behind > 0;
}

/** `mr_state` values that describe a request still open for review. */
const OPEN_REQUEST_STATES = ['open', 'draft'];

/**
 * Whether counting this feature's drift is worth two `git` calls.
 *
 * The complaint the signal exists for is "a pull request goes stale while I
 * merge other features", so the rows that earn the count are the ones holding
 * an open request. Counting every feature a project ever ran would spend the
 * calls on branches nobody is going to merge, and over SSH each one is a
 * round trip.
 */
export function holdsOpenRequest(feature: {
  mr_url?: string | null;
  mr_state?: string | null;
}): boolean {
  return (
    !!feature.mr_url &&
    OPEN_REQUEST_STATES.includes((feature.mr_state ?? '').toLowerCase())
  );
}
