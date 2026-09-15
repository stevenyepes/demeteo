import { ticketLabel } from './discoveryProgress';
import type { Ticket } from '../types';

/**
 * Base-branch choice, as pure functions of what the header and the new-Discovery
 * modal already have — so the lock predicate and the display/submit logic are
 * reachable from a test without rendering either surface.
 */

export interface BaseBranchLock {
  locked: boolean;
  /** Human-readable reason, non-null iff `locked`. Anticipates
   *  `base_branch_lock_refusal` — not a copy of its exact wording, since the
   *  backend's string is the source of truth if the two ever need comparing. */
  reason: string | null;
}

/** Locks once any ticket has left `unstarted` — started or dropped, mirroring
 *  `base_branch_lock_refusal`'s `state != Unstarted`, not `state == started`. */
export function baseBranchLock(tickets: readonly Ticket[]): BaseBranchLock {
  const left = tickets.filter((ticket) => ticket.state !== 'unstarted');
  if (left.length === 0) {
    return { locked: false, reason: null };
  }
  const labels = left.map((ticket) => ticketLabel(ticket.seq));
  const plural = labels.length > 1;
  const subject = plural ? `Tickets ${labels.join(', ')}` : `Ticket ${labels[0]}`;
  const be = plural ? 'are' : 'is';
  const pronoun = plural ? 'they are' : 'it is';
  return {
    locked: true,
    reason: `${subject} ${be} no longer Unstarted — the base branch is locked until ${pronoun} back to Unstarted.`,
  };
}

/** `baseBranch` verbatim when set; otherwise a sentence naming the project
 *  default — `defaultBranchName` when known, a generic phrase (matching
 *  `OriginPicker`'s "the project's default branch") when not yet fetched.
 *  Never returns `''`. */
export function baseBranchLabel(
  baseBranch: string | null,
  defaultBranchName: string | null,
): string {
  if (baseBranch !== null) return baseBranch;
  return defaultBranchName !== null
    ? `Starts from ${defaultBranchName}`
    : "Starts from the project's default branch";
}

/** What to pass as `setDiscoveryBase`'s second argument: `null` for the
 *  project default, the trimmed name otherwise (also `null` if that trims to
 *  empty — an unnamed "named" choice is not a request to change anything). */
export function resolveBaseBranchInput(useProjectDefault: boolean, name: string): string | null {
  if (useProjectDefault) return null;
  const trimmed = name.trim();
  return trimmed.length > 0 ? trimmed : null;
}
