import { ticketLabel } from './discoveryProgress';
import type { Ticket } from '../types';

/**
 * Base-branch choice, as pure functions of what the header and the new-Discovery
 * modal already have — so the lock predicate and the display/submit logic are
 * reachable from a test without rendering either surface.
 */

export interface BaseBranchLock {
  locked: boolean;
  /** Every ticket holding the lock, as `DSC-n` labels; empty iff unlocked. */
  tickets: string[];
  /** One line, non-null iff `locked`. It names the tickets only while there
   *  are few enough to read — a long-running discovery holds dozens, and a
   *  paragraph of ids pushed the graph down without saying anything a count
   *  does not. `listed` says whether the names are in it, so the surface knows
   *  whether to offer them separately. Anticipates `base_branch_lock_refusal`
   *  without copying its wording; the backend's string is the source of truth. */
  summary: string | null;
  listed: boolean;
}

/** Above this many, the summary counts the tickets instead of naming them. */
export const LOCK_NAMES_INLINE = 3;

/** Locks once any ticket has left `unstarted` — started or dropped, mirroring
 *  `base_branch_lock_refusal`'s `state != Unstarted`, not `state == started`. */
export function baseBranchLock(tickets: readonly Ticket[]): BaseBranchLock {
  const labels = tickets
    .filter((ticket) => ticket.state !== 'unstarted')
    .map((ticket) => ticketLabel(ticket.seq));
  if (labels.length === 0) {
    return { locked: false, tickets: [], summary: null, listed: false };
  }
  const listed = labels.length <= LOCK_NAMES_INLINE;
  let summary: string;
  if (labels.length === 1) {
    summary = `Base branch locked: ${labels[0]} has left Unstarted.`;
  } else if (listed) {
    summary = `Base branch locked: ${labels.join(', ')} have left Unstarted.`;
  } else {
    summary = `Base branch locked: ${labels.length} tickets have left Unstarted.`;
  }
  return { locked: true, tickets: labels, summary, listed };
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
