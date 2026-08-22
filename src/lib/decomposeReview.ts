import type { RunStatusTone } from './runStatus';
import { ticketLabel } from './discoveryProgress';
import type {
  ChangeKind,
  DecomposeProposal,
  ImmutableViolation,
  LockedTicket,
  ProposedChange,
} from '../types';

/**
 * What the proposed-changes modal decides before it draws anything
 * (`DISCOVERY_UI_SPEC.md` §4), kept out of the component so every rule below
 * is reachable from a test with no webview.
 *
 * **Nothing here caches the proposal.** §5.3 asks for a review, not a second
 * table, so `discovery_decompose` hands the plan out and
 * `discovery_apply_decomposition` takes it straight back — and re-resolves it
 * against the rows as they stand at that moment. A ticket can be started while
 * the modal is open; that is expected, and it is answered server-side by a
 * refusal rather than here by keeping the proposal fresh.
 */

/** One group of the modal's body, in the order §4.3 lists them. */
export interface ChangeGroup {
  kind: ChangeKind;
  label: string;
  tone: RunStatusTone;
  changes: ProposedChange[];
  /** The Fira Code count beside the label. */
  count: string;
}

const GROUPS: readonly { kind: ChangeKind; label: string; tone: RunStatusTone }[] = [
  { kind: 'added', label: 'Added', tone: 'emerald' },
  { kind: 'revised', label: 'Revised', tone: 'amber' },
  { kind: 'removed', label: 'Removed', tone: 'ruby' },
];

/**
 * The three groups, empty ones dropped.
 *
 * An empty group is not a state worth drawing: unlike the board's five lanes,
 * which are a fixed vocabulary a user reads positionally, a group here is a
 * list of things to decide about and a heading over nothing decides nothing.
 */
export function groupChanges(changes: readonly ProposedChange[]): ChangeGroup[] {
  return GROUPS.map((group) => {
    const members = changes.filter((change) => change.kind === group.kind);
    return { ...group, changes: members, count: groupCount(group.kind, members.length) };
  }).filter((group) => group.changes.length > 0);
}

/** §4.3's count copy. *Removed* and *dropped* are different words for
 *  different things (§4.7), so nothing here says either about the other. */
export function groupCount(kind: ChangeKind, n: number): string {
  const noun = n === 1 ? 'ticket' : 'tickets';
  if (kind === 'added') return `${n} new ${noun}`;
  return `${n} unstarted ${noun}`;
}

/** §4.8's count. */
export function lockedCount(n: number): string {
  return n === 1 ? '1 ticket has a feature' : `${n} tickets have a feature`;
}

/** Every change starts checked (§4.4). */
export function initialAccepted(changes: readonly ProposedChange[]): ReadonlySet<string> {
  return new Set(changes.map((change) => change.id));
}

/** The whole card is the click target, so this is what the click does. */
export function toggleAccepted(accepted: ReadonlySet<string>, id: string): ReadonlySet<string> {
  const next = new Set(accepted);
  if (!next.delete(id)) next.add(id);
  return next;
}

/**
 * The live footer label (§4.2).
 *
 * The denominator is the *diff*, not the plan: an unchanged ticket is nothing
 * to decide about, and the backend leaves it out of `changes` for exactly that
 * reason.
 */
export function applyLabel(acceptedCount: number, total: number): string {
  return `Apply ${acceptedCount} of ${total} changes`;
}

/** §4.2's eyebrow. Derived from the pass, never from a stored counter. */
export function passEyebrow(firstPass: boolean): string {
  return firstPass ? 'First pass' : 'Second pass';
}

/** The one validation surface the modal draws (§4.2). */
export interface ValidationState {
  tone: RunStatusTone;
  chip: string;
  sentence: string;
  /** The refusals themselves, rendered in mono under the sentence. */
  details: string[];
  /** Nothing in this proposal can be applied. */
  fatal: boolean;
}

/**
 * What the validation bar says.
 *
 * Three readings of one field pair. `refused` holds every refusal the pass was
 * re-asked over *including the ones it then fixed*, which is the bar's happy
 * sentence — the cycle was caught while the agent still had the graph in
 * context, so nothing invalid ever reached a row. `refusal` is set only when
 * the last attempt was refused too, and then nothing here can be applied at
 * all.
 */
export function validationState(proposal: DecomposeProposal): ValidationState {
  if (proposal.refusal !== null || proposal.violations.length > 0) {
    return {
      tone: 'ruby',
      chip: 'Schema refused',
      sentence:
        'The interviewer was asked again and did not answer with a plan this pass could use. Nothing invalid reaches a ticket row — and nothing here can be applied. Keep talking, or decompose again.',
      details: proposal.refusal === null ? [] : [proposal.refusal],
      fatal: true,
    };
  }

  const n = proposal.refused.length;
  if (n === 0) {
    return {
      tone: 'emerald',
      chip: 'Schema valid',
      sentence:
        'The plan was validated while the interviewer still had the graph in context. Nothing invalid reaches a ticket row.',
      details: [],
      fatal: false,
    };
  }
  return {
    tone: 'emerald',
    chip: 'Schema valid',
    sentence:
      n === 1
        ? 'One refusal was answered while the interviewer still had the graph in context — it re-authored the plan rather than shipping it. Nothing invalid reaches a ticket row.'
        : `${n} refusals were answered while the interviewer still had the graph in context — it re-authored the plan rather than shipping it. Nothing invalid reaches a ticket row.`,
    details: proposal.refused,
    fatal: false,
  };
}

/**
 * What the workspace says about a pass nobody has reviewed yet.
 *
 * Reads the same two fields {@link validationState} does and reaches the same
 * verdict in one line, because the two must never disagree: a notice promising
 * changes over a pass whose plan was refused would be offering something the
 * modal then declines to apply.
 */
export function pendingProposalNote(proposal: DecomposeProposal): string {
  if (validationState(proposal).fatal) {
    return 'A decompose pass finished without a plan that could be used. Nothing here can be applied.';
  }
  const n = proposal.changes.length;
  if (n === 0) return 'A decompose pass finished with nothing to change.';
  return n === 1
    ? 'A decompose pass is waiting for review: 1 proposed change.'
    : `A decompose pass is waiting for review: ${n} proposed changes.`;
}

/** A started ticket the pass tried to touch, attached to the locked card it
 *  names rather than to a sentence of its own (§4.9). */
export function violationFor(
  locked: LockedTicket,
  violations: readonly ImmutableViolation[],
): ImmutableViolation | null {
  return violations.find((violation) => violation.id === locked.id) ?? null;
}

/** What one Added or Removed card shows where a Revised card shows its diff. */
export function dependencyChip(change: ProposedChange, seqOf: (id: string) => number | null): {
  label: string;
  tone: RunStatusTone;
} {
  if (change.blocked_by.length === 0) {
    return { label: 'no prerequisites', tone: 'slate' };
  }
  const labels = change.blocked_by.map((id) => {
    const seq = seqOf(id);
    return seq === null ? id : ticketLabel(seq);
  });
  return { label: `blocked by ${labels.join(', ')}`, tone: 'amber' };
}

/**
 * Which checkboxes a refusal of the *subset* is about.
 *
 * A subset of a valid proposal is not itself valid: declining a new ticket
 * that another accepted one is `blocked_by`, or accepting a removal something
 * still waits on, leaves an edge pointing at nothing. The backend refuses that
 * — `validate_ticket_graph` over the resulting plan — and names the tickets in
 * single quotes, in proposal space, which is the same space the checkboxes are
 * keyed in. Matching them back is what turns a sentence at the bottom of a
 * modal into a mark on the two cards that caused it.
 *
 * Nothing here re-implements the rule. The frontend cannot decide which
 * combinations are legal without becoming a second authority on the graph, and
 * §5.2 keeps that authority in one place; this only reads the answer.
 */
export function refusedChangeIds(
  message: string,
  changes: readonly ProposedChange[],
): ReadonlySet<string> {
  const quoted = new Set(Array.from(message.matchAll(/'([^']+)'/g), (match) => match[1]));
  return new Set(changes.map((change) => change.id).filter((id) => quoted.has(id)));
}

/** The ticket number a card shows, or the placeholder an addition wears: `seq`
 *  is assigned at apply and never reissued, so a proposed ticket has no number
 *  yet and inventing one would name a row that does not exist. */
export function changeLabel(change: ProposedChange): string {
  return change.seq === null ? 'new' : ticketLabel(change.seq);
}

/**
 * §4.2's footer line, with the range it names derived rather than fixed.
 *
 * The mock spells `DSC-1 through DSC-7` because that is the plan it was drawn
 * over. The claim is about the stored numbers this apply will leave alone, so
 * it is read off them: a pass with nothing stored yet has no range to name and
 * says the property without one.
 */
export function renumberNote(proposal: DecomposeProposal): string {
  const stored = [
    ...proposal.locked.map((locked) => locked.seq),
    ...proposal.changes.map((change) => change.seq).filter((seq): seq is number => seq !== null),
  ];
  if (stored.length === 0) return 'Ticket ids are stable. Applying this renumbers nothing.';
  const low = Math.min(...stored);
  const high = Math.max(...stored);
  return low === high
    ? `Ticket ids are stable. Applying this never renumbers ${ticketLabel(low)}.`
    : `Ticket ids are stable. Applying this never renumbers ${ticketLabel(low)} through ${ticketLabel(high)}.`;
}
