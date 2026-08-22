import { invoke } from "@tauri-apps/api/core";
import type {
  DecomposeApply,
  DecomposeProposal,
  Discovery,
  DiscoveryBoard,
  DiscoveryDetail,
  DiscoveryMessage,
  DiscoverySummary,
  EffortLevel,
  Feature,
  Ticket,
  TicketEdit,
} from "../types";
import {
  attachmentWire,
  type AttachedFile,
  type AttachmentInput,
  type StagedAttachmentInput,
} from "./attachments";

/**
 * Typed IPC wrappers for Discovery and its Tickets — the commands in
 * `src-tauri/src/commands/discovery.rs` and `src-tauri/src/commands/tickets.rs`.
 *
 * **Rejections are plain strings.** Every command here returns
 * `Result<T, String>`, not the `AppError` envelope `commands/features.rs`
 * uses, so `asAppError` matches none of them and a caller wanting to show one
 * goes through `formatError`.
 */

/**
 * Mirrors `discovery_list` — every Discovery in a project, open or closed,
 * each carrying the two numbers its card renders that the row does not hold:
 * how many turns have been taken, and the ticket counter.
 *
 * The counter is the one `discovery_board` derives, so the card and the
 * workspace a click later cannot disagree about how much is done.
 */
export async function listDiscoveries(projectId: string): Promise<DiscoverySummary[]> {
  return invoke<DiscoverySummary[]>("discovery_list", { projectId });
}

/**
 * The summary a Discovery has the instant it is created, so the list can show
 * it without a second round trip.
 *
 * Both numbers are facts about a Discovery that has just opened rather than
 * placeholders: no turn has been taken, and nothing has been proposed.
 */
export function summaryOfNew(discovery: Discovery): DiscoverySummary {
  return {
    ...discovery,
    message_count: 0,
    progress: { blocked: 0, ready: 0, in_flight: 0, landed: 0, dropped: 0, live: 0 },
  };
}

/** Mirrors `discovery_get` — the Discovery and its whole transcript. */
export async function getDiscovery(discoveryId: string): Promise<DiscoveryDetail> {
  return invoke<DiscoveryDetail>("discovery_get", { discoveryId });
}

/**
 * Mirrors `discovery_create`. No worktree and no agent process yet — both wait
 * for the first turn that needs them.
 *
 * `machineId` is the picker's value: §4.5 makes the host part of the
 * interviewer choice, and a blank one is the same as none, which takes the
 * project's own host. A machine nothing is configured for is refused here
 * rather than three screens later, while the user is still looking at the
 * control they set it with.
 *
 * `stagedAttachments` land before the row is handed back, so the first turn a
 * user can take already sees every file — the ordering `start_feature` exists
 * to guarantee, one aggregate over.
 */
export async function createDiscovery(input: {
  projectId: string;
  title: string;
  agentKind: string;
  model: string | null;
  effort: EffortLevel | null;
  machineId: string | null;
  stagedAttachments?: StagedAttachmentInput[];
}): Promise<Discovery> {
  return invoke<Discovery>("discovery_create", {
    input: {
      project_id: input.projectId,
      title: input.title,
      agent_kind: input.agentKind,
      model: input.model,
      effort: input.effort,
      machine_id: input.machineId,
      staged_attachments: input.stagedAttachments ?? [],
    },
  });
}

/**
 * Mirrors `discovery_send_turn`. Returns as soon as the user's message is
 * stored — before the turn is set up — and the interviewer's half arrives over
 * `discovery_agent_event`, ending with `discovery_turn_completed`, because
 * leaving mid-interview is the case this feature exists for.
 *
 * A rejection therefore means the turn was never accepted, which includes a
 * refusal because one is already running; a setup that failed reaches the
 * surface as a `discovery_turn_status` of `error` instead.
 */
export async function sendDiscoveryTurn(
  discoveryId: string,
  text: string,
): Promise<DiscoveryMessage> {
  return invoke<DiscoveryMessage>("discovery_send_turn", { discoveryId, text });
}

/**
 * Mirrors `discovery_add_attachment`. A file is added to the Discovery
 * *before* the turn that talks about it, never passed to one: attachments are
 * owned by the interview, not by a turn, which is what keeps the composer's
 * chip row standing after the turn that added it (§4.6).
 */
export async function addDiscoveryAttachment(
  discoveryId: string,
  input: AttachmentInput,
): Promise<AttachedFile> {
  const wire = await attachmentWire(input);
  return invoke<AttachedFile>("discovery_add_attachment", {
    discoveryId,
    sourcePath: wire.sourcePath,
    mime: wire.mime,
    sourceFilename: wire.sourceFilename,
    bytes: wire.bytes,
  });
}

/** Mirrors `discovery_remove_attachment`. Idempotent. */
export async function removeDiscoveryAttachment(
  discoveryId: string,
  attachmentId: string,
): Promise<void> {
  return invoke<void>("discovery_remove_attachment", { discoveryId, attachmentId });
}

/** Mirrors `discovery_cancel_turn`. What the turn spent is still billed and
 *  whatever it managed to say is still stored — both already happened.
 *
 *  Nothing calls this while a turn reads `setting_up`: there is no agent
 *  session to cancel yet, so it would succeed and do nothing. See
 *  `application/discovery/mod.rs`. */
export async function cancelDiscoveryTurn(discoveryId: string): Promise<void> {
  return invoke<void>("discovery_cancel_turn", { discoveryId });
}

/** Mirrors `discovery_close`. Ends the interview and nothing else: the
 *  transcript and the tickets stay. */
export async function closeDiscovery(discoveryId: string): Promise<void> {
  return invoke<void>("discovery_close", { discoveryId });
}

/** Mirrors `discovery_reopen`. */
export async function reopenDiscovery(discoveryId: string): Promise<void> {
  return invoke<void>("discovery_reopen", { discoveryId });
}

/** Mirrors `discovery_reclaim_idle_worktrees`. Returns the ids it reclaimed. */
export async function reclaimIdleDiscoveryWorktrees(
  projectId: string,
  idleAfterMs: number,
): Promise<string[]> {
  return invoke<string[]>("discovery_reclaim_idle_worktrees", { projectId, idleAfterMs });
}

/** Mirrors `discovery_delete`. Refuses while any ticket has been started —
 *  those runs own branches, worktrees and pull requests that outlive the plan. */
export async function deleteDiscovery(discoveryId: string): Promise<void> {
  return invoke<void>("discovery_delete", { discoveryId });
}

/** Mirrors `discovery_board` — the tickets and the lanes they derive, from
 *  one call, so the graph and the board cannot disagree. */
export async function getDiscoveryBoard(discoveryId: string): Promise<DiscoveryBoard> {
  return invoke<DiscoveryBoard>("discovery_board", { discoveryId });
}

/**
 * Mirrors `discovery_decompose`. Asks the interviewer for a plan and hands
 * back what applying it *would* change; nothing is written.
 *
 * The pass streams through the same three events a turn does, so the surface
 * can show the agent working — but the proposal itself arrives on the call,
 * because there is nothing to render until it is whole.
 *
 * It also arrives on `DiscoveryDetail.pending_proposal`, which is what makes
 * this promise safe to lose: the pass is billed and takes minutes, and the
 * view that awaits it can be navigated away from long before it answers.
 */
export async function decomposeDiscovery(discoveryId: string): Promise<DecomposeProposal> {
  return invoke<DecomposeProposal>("discovery_decompose", { discoveryId });
}

/**
 * Mirrors `discovery_discard_proposal` — forget the pass awaiting review.
 *
 * Closing the review does not do this. A proposal is billed work, so leaving
 * it keeps it where the workspace can offer it again; this is the press that
 * says otherwise.
 */
export async function discardProposal(discoveryId: string): Promise<void> {
  return invoke<void>("discovery_discard_proposal", { discoveryId });
}

/**
 * Mirrors `discovery_apply_decomposition` — land the checked changes and
 * return the board they leave behind.
 *
 * `tickets` goes back exactly as {@link decomposeDiscovery} handed it over.
 * The stored proposal is a view awaiting review and never an answer: the
 * backend re-resolves and re-diffs it against the rows *as they stand now*, so
 * a ticket started since the pass ran is refused here rather than silently
 * rewritten — which is why nothing on this side polls or refetches to keep a
 * proposal fresh. Applying clears what was stored.
 */
export async function applyDecomposition(input: DecomposeApply): Promise<DiscoveryBoard> {
  return invoke<DiscoveryBoard>("discovery_apply_decomposition", { input });
}

/**
 * Mirrors `ticket_update`.
 *
 * Takes the whole {@link TicketEdit} because every key of it is required on
 * the wire: serde reads an absent key and an explicit `null` the same way, so
 * a partial payload would quietly mean *keep* where the user meant *clear*.
 *
 * Returns the board rather than the row — an edited edge moves the standing of
 * everything under it, so a caller patching one row locally would hold a board
 * that disagrees with it.
 */
export async function updateTicket(
  ticketId: string,
  edit: TicketEdit,
): Promise<DiscoveryBoard> {
  return invoke<DiscoveryBoard>("ticket_update", { ticketId, edit });
}

/** Mirrors `ticket_briefing` — what the ticket's agent will be told, rendered
 *  before anything starts. */
export async function getTicketBriefing(ticketId: string): Promise<string> {
  return invoke<string>("ticket_briefing", { ticketId });
}

/** Mirrors `ticket_start`. */
export async function startTicket(ticketId: string): Promise<Feature> {
  return invoke<Feature>("ticket_start", { ticketId });
}

/** Mirrors `ticket_force_start`. The reason is not decoration: it reaches the
 *  agent in its own prerequisite briefing, which is what stops a bypass from
 *  becoming an unexplained one. */
export async function forceStartTicket(ticketId: string, reason: string): Promise<Feature> {
  return invoke<Feature>("ticket_force_start", { ticketId, reason });
}

/** Mirrors `ticket_drop`. */
export async function dropTicket(ticketId: string, reason: string): Promise<Ticket> {
  return invoke<Ticket>("ticket_drop", { ticketId, reason });
}

/**
 * Mirrors `ticket_add_attachment`. Staged on the Ticket and committed to the
 * Feature the moment it starts (§9.3), so a Ticket that never starts never
 * writes an attachment row.
 *
 * Takes the pick rather than the wire shape, exactly as
 * {@link addDiscoveryAttachment} does: `attachmentWire` is where the
 * bytes-vs-path question is answered, and a second owner answering it for
 * itself is how the two come to disagree.
 */
export async function addTicketAttachment(
  ticketId: string,
  input: AttachmentInput,
): Promise<AttachedFile> {
  const wire = await attachmentWire(input);
  return invoke<AttachedFile>("ticket_add_attachment", {
    ticketId,
    sourcePath: wire.sourcePath,
    mime: wire.mime,
    sourceFilename: wire.sourceFilename,
    bytes: wire.bytes,
  });
}

/** Mirrors `ticket_remove_attachment`. */
export async function removeTicketAttachment(
  ticketId: string,
  attachmentId: string,
): Promise<void> {
  return invoke<void>("ticket_remove_attachment", { ticketId, attachmentId });
}

// ── The streaming contract (`application/discovery/events.rs`) ─────────────

/** Every `AgentEvent` of a turn, as it arrives. */
export const EVENT_DISCOVERY_AGENT_EVENT = "discovery_agent_event";
/** A turn's phase: `setting_up`, then `running`, then `idle` or `error`.
 *  Read through `phaseOfStatus` in `lib/discoveryActivity.ts` — which of them
 *  leave a turn live is a decision, not a string comparison. */
export const EVENT_DISCOVERY_TURN_STATUS = "discovery_turn_status";
/** The completion signal — a multi-minute turn that ended silently would
 *  force the user to sit and watch it. */
export const EVENT_DISCOVERY_TURN_COMPLETED = "discovery_turn_completed";

/**
 * Payload of `discovery_agent_event`.
 *
 * `event` is the Rust `AgentEvent`, an enum serde-tagged by `kind` and wide
 * enough that mirroring the whole of it here would be a second copy to keep in
 * step for the sake of the three variants this surface reads. It stays
 * `unknown` behind the guards in `lib/discoveryActivity.ts`.
 */
export interface DiscoveryAgentEventPayload {
  discovery_id: string;
  event: unknown;
}

/** Mirrors `DiscoveryTurnStatus`. */
export interface DiscoveryTurnStatusPayload {
  discovery_id: string;
  status: string;
  reason: string | null;
}

/** Mirrors `DiscoveryTurnCompleted`. */
export interface DiscoveryTurnCompletedPayload {
  discovery_id: string;
  title: string;
  message_id: string | null;
  ending: "success" | "interrupted" | "failed" | "environmental";
  reason: string | null;
  cost_usd: number;
  tokens: number;
  duration_ms: number;
  /** Whether the turn had to carry the transcript itself, rather than resume
   *  the harness's own copy of the session. */
  reseeded: boolean;
  nothing_left_to_settle: boolean;
}

