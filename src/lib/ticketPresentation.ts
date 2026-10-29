import type { RunStatusTone } from './runStatus';
import { prNumber, ticketLabel } from './discoveryProgress';
import type { TicketLane, TicketView } from '../types';

/**
 * One bucket, one vocabulary — the tone, the lane, the note and the verdict a
 * ticket wears, derived from the standing `discovery_board` already computed.
 *
 * `DISCOVERY_UI_SPEC.md` §6.2 asks for exactly one of these: the graph nodes,
 * the board lanes, the chips, the verdict card and the legend all read the
 * same bucket, and a second mapping is how two of those come to disagree about
 * a ticket nothing changed.
 *
 * Nothing here is stored. `docs/PRD_DISCOVERY.md` §6.3 has the argument — a
 * stored status is a cache of a derived fact, and it drifts the moment
 * something changes through a path the updater did not observe.
 */

export type TicketIndex = ReadonlyMap<string, TicketView>;

export function indexTickets(tickets: readonly TicketView[]): TicketIndex {
  return new Map(tickets.map((view) => [view.ticket.id, view]));
}

export interface LaneMeta {
  lane: TicketLane;
  label: string;
  /** Right of the lane rule, and the legend has no counterpart for it. */
  note: string;
  tone: RunStatusTone;
}

/**
 * All five, always. Ready leads because it is the lane a person acts on, and
 * it is the one amber lane for the same reason: a ready ticket waits for
 * someone to start it, which is exactly what amber means (AGENTS.md §4). The
 * spec painted Ready violet; decision 61 took violet out of statuses, and the
 * slate it fell back to made a startable ticket read as a blocked one.
 * Blocked is slate whatever its prerequisites are doing — nobody can act on
 * it, and its note already names what it waits on.
 */
export const TICKET_LANES: readonly LaneMeta[] = [
  { lane: 'ready', label: 'Ready', note: 'you start these', tone: 'amber' },
  { lane: 'in_flight', label: 'In flight', note: 'PR open', tone: 'cyan' },
  { lane: 'blocked', label: 'Blocked', note: 'waiting on an edge', tone: 'slate' },
  { lane: 'landed', label: 'Landed', note: 'merged into master', tone: 'emerald' },
  { lane: 'dropped', label: 'Dropped', note: 'decided against, with a reason', tone: 'slate' },
];

/** The legend and lane-header dot. Blocked and dropped share a tone, so the
 *  dot is what tells them apart at a glance. */
export const LANE_DOT: Record<TicketLane, string> = {
  ready: 'bg-amber-400',
  in_flight: 'bg-cyan-400',
  blocked: 'bg-slate-400',
  landed: 'bg-emerald-400',
  dropped: 'bg-slate-600',
};

/** Group every ticket into its lane, empty lanes included. */
export function bucketByLane(
  tickets: readonly TicketView[],
): { meta: LaneMeta; tickets: TicketView[] }[] {
  return TICKET_LANES.map((meta) => ({
    meta,
    tickets: tickets.filter((view) => view.standing.lane === meta.lane),
  }));
}

export function ticketTone(view: TicketView): RunStatusTone {
  return LANE_TONE[view.standing.lane];
}

const LANE_TONE = Object.fromEntries(
  TICKET_LANES.map((meta) => [meta.lane, meta.tone]),
) as Record<TicketLane, RunStatusTone>;

/** The lane's own word for a ticket sitting in it. */
export function stateLabel(view: TicketView): string {
  switch (view.standing.lane) {
    case 'landed':
      return 'Landed';
    case 'in_flight':
      return 'Running';
    case 'ready':
      return 'Ready';
    case 'blocked':
      return 'Blocked';
    case 'dropped':
      return wasDropped(view) ? 'Dropped' : 'Closed';
  }
}

/**
 * A dropped ticket and a closed-unmerged one share a lane and not a note.
 *
 * `docs/TASKS_DISCOVERY.md` ("Where a closed-unmerged ticket shows") is
 * explicit: the lane's own note reads *decided against, with a reason*, and a
 * PR that closed has no such reason. `null` rather than a stand-in — a card
 * must never render an absent reason as though it had one.
 */
export function dropNote(view: TicketView): string | null {
  if (wasDropped(view)) return view.ticket.drop_reason;
  const number = prNumber(view.feature?.mr_url ?? null);
  return number ? `PR #${number} closed without merging` : 'its PR closed without merging';
}

/** The Fira Code line under a node or a board card. */
export function ticketNote(view: TicketView, index: TicketIndex): string | null {
  const number = prNumber(view.feature?.mr_url ?? null);
  switch (view.standing.lane) {
    case 'landed':
      return number ? `PR #${number} merged` : 'merged into master';
    case 'in_flight':
      return number ? `PR #${number} open` : 'started · no PR yet';
    case 'ready':
      return view.ticket.blocked_by.length > 0 ? 'every prerequisite landed' : 'nothing gates it';
    case 'blocked':
      return `waiting on ${blockerLabels(view, index).join(', ')}`;
    case 'dropped':
      return dropNote(view);
  }
}

export interface Verdict {
  label: string;
  tone: RunStatusTone;
  why: string;
}

/**
 * The inspector's verdict card. Every string here is recomputed from the
 * edges on the way past — §6.7 keeps the copy saying so, and keeps Demeteo
 * *saying* a ticket is startable rather than claiming it starts one.
 */
export function verdict(view: TicketView, index: TicketIndex): Verdict {
  const tone = ticketTone(view);
  switch (view.standing.lane) {
    case 'landed':
      return {
        label: 'Started',
        tone,
        why: 'Its PR merged into master. Read from the forge, not from the run.',
      };
    case 'in_flight':
      return {
        label: 'Started',
        tone,
        why: 'Running now. Its PR is open, so nothing waiting on it has been released yet.',
      };
    case 'ready':
      return {
        label: 'Startable',
        tone,
        why: `${releasedClause(view)} Demeteo says so; it does not start anything on its own.`,
      };
    case 'dropped':
      return wasDropped(view)
        ? {
            label: 'Dropped',
            tone,
            why: 'Dropped with a reason, which satisfies its dependents the same way a closed PR does. The record of the decision stays.',
          }
        : {
            label: 'Closed',
            tone,
            why: 'Its PR closed without merging, which satisfies its dependents the same way a drop does. The record of the decision stays.',
          };
    case 'blocked':
      return { label: 'Blocked', tone, why: blockedWhy(view, index) };
  }
}

export type TicketActionKind = 'open' | 'start' | 'none';

export interface TicketAction {
  label: string;
  kind: TicketActionKind;
  disabled: boolean;
}

/** The inspector's primary button, per §3.6.8. */
export function primaryAction(view: TicketView, index: TicketIndex): TicketAction {
  if (view.ticket.state === 'started') {
    return { label: 'Open feature', kind: 'open', disabled: view.feature === null };
  }
  if (view.ticket.state === 'dropped') return { label: 'Dropped', kind: 'none', disabled: true };
  if (view.standing.startable) return { label: 'Start ticket', kind: 'start', disabled: false };

  const labels = blockerLabels(view, index);
  return {
    label: labels.length === 1 ? `Blocked by ${labels[0]}` : `Blocked by ${labels.length} tickets`,
    kind: 'none',
    disabled: true,
  };
}

/** Force start bypasses edges, so it is offered only where edges are what is
 *  in the way. */
export function showsForceStart(view: TicketView): boolean {
  return view.standing.lane === 'blocked';
}

export interface PrerequisiteRow {
  id: string;
  label: string;
  title: string;
  note: string | null;
  state: string;
  tone: RunStatusTone;
}

export function prerequisiteRows(view: TicketView, index: TicketIndex): PrerequisiteRow[] {
  return view.ticket.blocked_by.map((id) => {
    const prerequisite = index.get(id);
    if (!prerequisite) {
      return {
        id,
        label: '—',
        title: 'Unknown prerequisite',
        note: 'not in this discovery',
        state: 'Unknown',
        tone: 'slate' as const,
      };
    }

    const number = prNumber(prerequisite.feature?.mr_url ?? null);
    const base = {
      id,
      label: ticketLabel(prerequisite.ticket.seq),
      title: prerequisite.ticket.title,
    };

    switch (prerequisite.standing.lane) {
      case 'landed':
        return {
          ...base,
          note: number ? `PR #${number} merged into master` : 'merged into master',
          state: 'Landed',
          tone: 'emerald' as const,
        };
      case 'in_flight':
        return {
          ...base,
          note: number ? `PR #${number} open` : 'started · no PR yet',
          state: 'Waiting',
          tone: 'cyan' as const,
        };
      case 'dropped':
        return {
          ...base,
          note: dropNote(prerequisite),
          state: stateLabel(prerequisite),
          tone: 'slate' as const,
        };
      default:
        return {
          ...base,
          note: 'not started — no PR to read',
          state: 'Waiting',
          tone: 'slate' as const,
        };
    }
  });
}

function wasDropped(view: TicketView): boolean {
  return view.ticket.state === 'dropped';
}

function blockerLabels(view: TicketView, index: TicketIndex): string[] {
  return view.standing.blockers.map((blocker) => {
    const prerequisite = index.get(blocker.id);
    return prerequisite ? ticketLabel(prerequisite.ticket.seq) : 'an unknown ticket';
  });
}

function releasedClause(view: TicketView): string {
  const total = view.ticket.blocked_by.length;
  if (total === 0) return 'Nothing in this discovery gates it.';
  if (total === 1) return 'Its one prerequisite merged.';
  return `All ${count(total)} of its prerequisites merged.`;
}

function blockedWhy(view: TicketView, index: TicketIndex): string {
  const total = view.ticket.blocked_by.length;
  const outstanding = view.standing.blockers.length;
  const released = total - outstanding;

  if (released > 0) {
    const verb = released === 1 ? 'has' : 'have';
    return `${capitalise(count(released))} of ${count(total)} prerequisites ${verb} landed. Recomputed from the edges on every read — there is no readiness column to drift.`;
  }
  if (outstanding === 1) {
    const [label] = blockerLabels(view, index);
    return `${label} has not started, so it has no PR to read a verdict from.`;
  }
  return `${capitalise(count(outstanding))} prerequisites, ${outstanding === 2 ? 'neither' : 'none'} started.`;
}

const WORDS = ['zero', 'one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight', 'nine'];

/** The mocks spell small counts as words ("One of two prerequisites"), so the
 *  derived sentence has to as well or it reads as a different voice. */
function count(n: number): string {
  return WORDS[n] ?? String(n);
}

function capitalise(word: string): string {
  return word.charAt(0).toUpperCase() + word.slice(1);
}
