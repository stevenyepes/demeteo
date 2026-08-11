/**
 * What the activity block's sync affordance says (`docs/UI_REDESIGN_PLAN.md`
 * §1 **D**, §5.2).
 *
 * The affordance exists because the two transports keep the feed current by
 * different means, and the old copy claimed one of them for both: a fixed
 * "polling every 3s" caption sat under the panel while the panel itself polled
 * at 2s, and a local run — which is pushed, never polled — never had a caption
 * at all. Deriving the words from the same interval that drives the poll is why
 * `pollMs` is an argument rather than a literal here.
 *
 * The collapsed case is the one worth keeping: closing the remote panel
 * unmounts the poll with it, so the feed genuinely stops advancing until it is
 * reopened. A local feed is pushed into a hook that outlives the panel, so
 * closing it hides rows that keep arriving. Same gesture, opposite consequence,
 * and the affordance is the only place a user can learn which one they got.
 */

import type { RunStatusTone } from './runStatus';

export type ActivityTransport = 'local' | 'remote';

export interface ActivitySyncInput {
  transport: ActivityTransport;
  /** The panel's disclosure state — a closed remote panel is not polling. */
  open: boolean;
  /** The run has reached a terminal status: the log cannot grow further. */
  terminal: boolean;
  /** Poll interval of the remote tail, in ms. Ignored for `local`. */
  pollMs: number;
  /** Consecutive failed polls, below the panel's error threshold. */
  consecutiveFailures?: number;
  /** A failure streak long enough that the panel is showing an error. */
  errored?: boolean;
}

export interface ActivitySync {
  label: string;
  tone: RunStatusTone;
  /** Whether the affordance's dot should pulse — the feed is advancing now. */
  live: boolean;
  /** Long-form reason, for the affordance's `title`. */
  hint: string;
}

/** Whole seconds, so a 2000 ms interval reads "2s" rather than "2.0s". */
function everyN(pollMs: number): string {
  return `every ${Math.round(pollMs / 1000)}s`;
}

export function activitySync(input: ActivitySyncInput): ActivitySync {
  const { transport, open, terminal, pollMs, consecutiveFailures = 0, errored = false } = input;

  if (transport === 'local') {
    if (terminal) {
      return { label: 'final', tone: 'slate', live: false, hint: 'The run finished — this log is complete.' };
    }
    return {
      label: 'live',
      tone: 'cyan',
      live: true,
      // Deliberately unchanged by `open`: the push feeds a hook above this
      // panel, so a closed panel is hiding rows, not missing them.
      hint: open
        ? 'Events are pushed from this machine as the run emits them.'
        : 'Events keep arriving while this is closed — reopen to read them.',
    };
  }

  if (terminal) {
    return { label: 'final', tone: 'slate', live: false, hint: 'The run finished — this log was fetched once and is complete.' };
  }
  if (!open) {
    return {
      label: 'paused',
      tone: 'slate',
      live: false,
      hint: 'Closed, so the tunnel is not being polled. Reopen to resume the tail.',
    };
  }
  if (errored) {
    return {
      label: 'disconnected',
      tone: 'ruby',
      live: false,
      hint: `Several polls in a row failed. Still retrying ${everyN(pollMs)}.`,
    };
  }
  if (consecutiveFailures > 0) {
    return {
      label: 'reconnecting',
      tone: 'amber',
      live: false,
      hint: `The last poll failed. Retrying ${everyN(pollMs)}.`,
    };
  }
  return {
    label: everyN(pollMs),
    tone: 'cyan',
    live: true,
    hint: `Tailing the runner's event log over the tunnel ${everyN(pollMs)}.`,
  };
}
