import type { RunStatusTone } from './runStatus';
import type { Discovery, DiscoveryBoard, TicketProgress, TicketView } from '../types';

/**
 * The derived readings a Discovery card and a Discovery workspace both show,
 * as pure functions of what the backend already computed
 * (`domain::ticket_graph::derive_board`).
 *
 * **One helper, two surfaces, deliberately.** `docs/PRD_DISCOVERY.md` §9.2
 * counts *landed against live tickets*, where live excludes dropped, and says
 * *landed* rather than *started*: a run that finished without merging is what
 * §6.4 refuses to call done, so a bar counting it would contradict the gate one
 * screen below it. The mocks disagree with themselves here —
 * `DISCOVERY_UI_SPEC.md` §3.5.1 records that Project Home's card reads
 * `1 of 7` for the same set the workspace reads `1 of 6`, and that the
 * dropped-excluding figure is the one to ship. Two call sites deriving that
 * separately is how the mocks' own disagreement gets rebuilt.
 */

/** The stable display id of a ticket — `Ticket.seq`, never a list index,
 *  because §5.3 forbids renumbering and the number is what a user says out
 *  loud. */
export function ticketLabel(seq: number): string {
  return `DSC-${seq}`;
}

/**
 * How many turns a Discovery has taken, as both surfaces say it.
 *
 * **A turn is one stored message** — one thing said, by either side. That is
 * the decision `docs/TASKS_DISCOVERY.md` left open, and it is settled this way
 * because it is the only reading both surfaces can reach: Project Home is
 * given `DiscoverySummary.message_count`, a `COUNT(*)` over
 * `discovery_messages`, and has no transcript to count anything else from.
 *
 * The rejected reading is the workspace's old one — *rendered blocks*, every
 * bubble and question card. A question is part of the message that asked it
 * (`docs/TASKS_DISCOVERY.md`, "The interview turn contract"): it is not a
 * second thing the interviewer said, it is how one thing it said is drawn. So
 * the two surfaces disagreed by exactly the number of questions asked, and the
 * larger number was counting a rendering decision.
 *
 * Both call sites now pass a count of stored messages, which is why this takes
 * a number rather than a transcript: nothing that has only blocks can call it.
 */
export function turnCountLabel(messages: number): string {
  return messages === 1 ? '1 turn' : `${messages} turns`;
}

/**
 * The progress readout, computed and never stored.
 *
 * Over the counter alone rather than the ticket set, because Project Home's
 * card is given only the counter (`DiscoverySummary.progress`) and reading it
 * a second way there is exactly the divergence this module exists to prevent.
 * Every ticket sits in one lane, so `live + dropped` *is* the size of the set.
 *
 * `null` when the Discovery has proposed nothing yet: there is no arithmetic
 * to report, and `0 of 0 landed` reads as a failure rather than as an
 * interview still in progress. A Discovery whose every ticket was dropped has
 * proposed something, and does read `0 of 0 landed`.
 *
 * The *in flight* clause is dropped at zero, which is how the mock renders a
 * finished Discovery (`5 of 5 landed`).
 */
export function progressText(progress: TicketProgress): string | null {
  const { landed, live, dropped, in_flight: inFlight } = progress;
  if (live + dropped === 0) return null;
  const head = `${landed} of ${live} landed`;
  return inFlight > 0 ? `${head} · ${inFlight} in flight` : head;
}

/** The two segments of the split bar, as percentages of the same *live*
 *  denominator the text uses. */
export function progressSegments(progress: TicketProgress): {
  landedPct: number;
  inFlightPct: number;
} {
  const { landed, live, in_flight: inFlight } = progress;
  if (live <= 0) return { landedPct: 0, inFlightPct: 0 };
  return {
    landedPct: (landed / live) * 100,
    inFlightPct: (inFlight / live) * 100,
  };
}

/** Where a Discovery is in its life, as a chip. */
export interface DiscoveryLifecycle {
  label: 'Interviewing' | 'Decomposed' | 'Closed';
  tone: RunStatusTone;
  /** Only a live turn pulses. A dot that pulses on stored state alone claims
   *  something is happening when nothing is. */
  live: boolean;
}

/**
 * Derived, because `DiscoveryStatus` stores only open/closed — the three rows
 * the mock draws are that pair crossed with whether anything has been proposed
 * and whether a turn is running right now.
 */
export function discoveryLifecycle(
  discovery: Discovery,
  ticketCount: number,
  turnRunning: boolean,
): DiscoveryLifecycle {
  if (discovery.status === 'closed') return { label: 'Closed', tone: 'slate', live: false };
  if (turnRunning) return { label: 'Interviewing', tone: 'violet', live: true };
  if (ticketCount > 0) return { label: 'Decomposed', tone: 'cyan', live: false };
  return { label: 'Interviewing', tone: 'violet', live: false };
}

/**
 * The card's detail line.
 *
 * Two clauses, both mechanically derivable from the board: which tickets
 * Demeteo *says* are startable, and the first one waiting on a published PR.
 * The wording is §9.4's — Demeteo says a ticket is startable and never starts
 * one — and it is why this reads "is startable now" rather than "can start".
 *
 * `null` rather than a filler sentence when neither clause applies. The mock's
 * other two rows are narrative summaries of a particular plan's history, which
 * nothing here can derive; inventing one would be a claim about a plan this
 * function has not read.
 */
export function discoveryDetailLine(board: DiscoveryBoard): string | null {
  const clauses: string[] = [];

  const startable = board.tickets.filter((t) => t.standing.startable);
  if (startable.length > 0) {
    const labels = startable.map((t) => ticketLabel(t.ticket.seq));
    clauses.push(
      labels.length === 1
        ? `${labels[0]} is startable now.`
        : `${joinLabels(labels)} are startable now.`,
    );
  }

  const waiting = firstWaitingOnPr(board.tickets);
  if (waiting) clauses.push(waiting);

  return clauses.length > 0 ? clauses.join(' ') : null;
}

function joinLabels(labels: string[]): string {
  if (labels.length <= 2) return labels.join(' and ');
  return `${labels.slice(0, -1).join(', ')} and ${labels[labels.length - 1]}`;
}

function firstWaitingOnPr(tickets: TicketView[]): string | null {
  const bySeq = new Map(tickets.map((t) => [t.ticket.id, t]));
  for (const ticket of tickets) {
    if (ticket.standing.lane !== 'blocked') continue;
    for (const blocker of ticket.standing.blockers) {
      const prerequisite = bySeq.get(blocker.id);
      const number = prNumber(prerequisite?.feature?.mr_url ?? null);
      if (number === null) continue;
      return `${ticketLabel(ticket.ticket.seq)} waits on PR #${number}.`;
    }
  }
  return null;
}

/**
 * The number a forge URL ends in. GitLab's path segment is spelled
 * `merge_requests` and GitHub's `pull`; the publisher adapters already
 * normalise the *state* vocabulary but not the URL, so both are matched here.
 * `null` for anything else, which is what an unpublished run has.
 */
export function prNumber(url: string | null): string | null {
  if (!url) return null;
  const match = /\/(?:pull|pulls|merge_requests|pull-requests)\/(\d+)/.exec(url);
  return match ? match[1] : null;
}
