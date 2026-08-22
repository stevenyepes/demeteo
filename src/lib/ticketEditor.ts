import type { EffortLevel } from './effortLevels';
import { ticketLabel } from './discoveryProgress';
import type { Ticket, TicketEdit, TicketView } from '../types';

/**
 * What the ticket editor drawer holds and what it saves
 * (`DISCOVERY_UI_SPEC.md` §5, `docs/PRD_DISCOVERY.md` §5.4), as pure functions
 * of the row — so the whole of the save payload is reachable from a test
 * without rendering a drawer.
 *
 * **The drawer holds the ticket whole and saves it whole.** Every key of
 * [`TicketEdit`] is required on the wire, because Rust reads an absent key and
 * an explicit `null` identically: a partial payload would turn *clear the
 * model* into *keep the model*, with nothing on either side to notice. There
 * is deliberately no patch shape anywhere below.
 */

/** The form's own state: strings where the wire has nullable columns, because
 *  an `<input>` produces `''` and never `null`. */
export interface TicketDraft {
  title: string;
  description: string;
  acceptance: string[];
  files: string[];
  /** Stored ticket ids, in the order the rows are drawn. */
  blockedBy: string[];
  testCommand: string;
  workflowId: string;
  agentKind: string;
  model: string;
  /** `''` is *unset*, which is a real choice: the run falls back to the
   *  project's default rather than to a level nobody picked. */
  effort: EffortLevel | '';
}

/** The row, as a form holds it. */
export function draftOf(ticket: Ticket): TicketDraft {
  return {
    title: ticket.title,
    description: ticket.description,
    acceptance: [...ticket.acceptance],
    files: [...ticket.files],
    blockedBy: [...ticket.blocked_by],
    testCommand: ticket.test_command ?? '',
    workflowId: ticket.workflow_id ?? '',
    agentKind: ticket.agent_kind ?? '',
    model: ticket.model ?? '',
    effort: ticket.effort ?? '',
  };
}

/**
 * The form, as the wire takes it.
 *
 * Blanks collapse to `null` here as well as in `TicketEdit::normalized` on the
 * far side. Both, deliberately: the backend owns the rule, and doing it here
 * too is what keeps the drawer from reporting *changed* on a field where a
 * user only moved the caret through an empty box.
 */
export function editOf(draft: TicketDraft): TicketEdit {
  return {
    title: draft.title.trim(),
    description: draft.description.trim(),
    acceptance: entries(draft.acceptance),
    files: entries(draft.files),
    blocked_by: entries(draft.blockedBy),
    test_command: chosen(draft.testCommand),
    workflow_id: chosen(draft.workflowId),
    agent_kind: chosen(draft.agentKind),
    model: chosen(draft.model),
    effort: draft.effort === '' ? null : draft.effort,
  };
}

/**
 * A locked ticket is not editable, and is shown as locked rather than allowed
 * to fail on save.
 *
 * Mirrors `application::tickets::is_locked`, and mirrors both halves of it:
 * §5.4 says a Ticket locks when it has a Feature, and `started` is the state
 * that says so — a row carrying either is one whose run is already working
 * against the plan as it stands.
 */
export function isTicketLocked(ticket: Ticket): boolean {
  return ticket.feature_id !== null || ticket.state === 'started';
}

/** Whether anything on the form differs from the row it was seeded with. */
export function isDirty(draft: TicketDraft, ticket: Ticket): boolean {
  return JSON.stringify(editOf(draft)) !== JSON.stringify(editOf(draftOf(ticket)));
}

/** §5.5's staged-attachment chip. The ceiling is `AttachmentDropzone`'s own. */
export const MAX_ATTACHMENTS = 10;

export function stagedCount(n: number): string {
  return `${n} of ${MAX_ATTACHMENTS} · staged`;
}

/** §5.9's rule: below this the reason is not one. */
export const MIN_REASON = 8;

/** One entry of the `Add an edge` picker. */
export interface EdgeOption {
  id: string;
  label: string;
  title: string;
}

/**
 * What this ticket may point at: every other ticket in the same Discovery.
 *
 * §6.2 closes the graph over one Discovery, so the candidate set is exactly
 * its siblings — and a ticket may not wait on itself. Whether the resulting
 * graph is *acyclic* is not decided here: `validate_ticket_graph` is the one
 * authority on that and refuses the save, because a second reading of the rule
 * on this side is a second answer waiting to drift.
 */
export function edgeOptions(
  ticket: Ticket,
  siblings: readonly TicketView[],
  chosenEdges: readonly string[],
): EdgeOption[] {
  const taken = new Set(chosenEdges);
  return siblings
    .filter((view) => view.ticket.id !== ticket.id && !taken.has(view.ticket.id))
    .map((view) => ({
      id: view.ticket.id,
      label: ticketLabel(view.ticket.seq),
      title: view.ticket.title,
    }));
}

function entries(items: readonly string[]): string[] {
  return items.map((item) => item.trim()).filter((item) => item.length > 0);
}

function chosen(value: string): string | null {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}
