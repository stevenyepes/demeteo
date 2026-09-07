import type { RunStatusTone } from './runStatus';
import type { AskThread } from '../types';

/**
 * Where an Ask thread is in its life, as a chip — the same three-part reading
 * `discoveryProgress.ts` derives for a Discovery, and deliberately the same
 * shape: the two lists sit on adjacent tabs of Project Home, so a thread that
 * is running must not be labelled by one vocabulary and an interview by
 * another.
 *
 * Derived, because `AskStatus` stores only open/closed. `Asking` is a turn
 * running *right now*, which nothing persists — `AskThreadSwitcher` records
 * why liveness can only come off the `ask_turn_status` stream.
 */
export interface AskLifecycle {
  label: 'Asking' | 'Answered' | 'Open' | 'Closed';
  tone: RunStatusTone;
  /** Only a live turn pulses. A dot that pulses on stored state alone claims
   *  something is happening when nothing is. */
  live: boolean;
}

export function askLifecycle(thread: AskThread, turnRunning: boolean): AskLifecycle {
  if (thread.status === 'closed') return { label: 'Closed', tone: 'slate', live: false };
  if (turnRunning) return { label: 'Asking', tone: 'violet', live: true };
  if (thread.turn_count > 0) return { label: 'Answered', tone: 'cyan', live: false };
  return { label: 'Open', tone: 'violet', live: false };
}

/** How many turns a thread has taken. `AskThread.turn_count` counts settled
 *  turns, not stored messages — the difference from
 *  `discoveryProgress.turnCountLabel`, which counts messages because Project
 *  Home is given nothing else for a Discovery. Same wording either way. */
export function turnCountLabel(turnCount: number): string {
  return turnCount === 1 ? '1 turn' : `${turnCount} turns`;
}
