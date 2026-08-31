import { invoke } from "@tauri-apps/api/core";
import type { AskMessage, AskThread, AskThreadDetail, EffortLevel } from "../types";

/**
 * Typed IPC wrappers for Ask — the commands in `src-tauri/src/commands/ask.rs`.
 *
 * Storage and lifecycle, plus sending a turn and the three events it streams
 * over (`application/ask/events.rs`).
 *
 * **Rejections are plain strings** — every command returns `Result<T, String>`,
 * the same convention `lib/discovery.ts` documents.
 */

/**
 * Mirrors `ask_create`. No worktree and no agent process yet — both wait for
 * the first turn that needs them.
 *
 * `machineId` is the picker's value, on the same terms as
 * {@link import("./discovery").createDiscovery}'s: a blank one is the same as
 * none, which takes the project's own host.
 *
 * `network` is the thread's opening web-access posture. It has to be settled
 * here rather than by a later {@link updateAskThreadSettings}, because the
 * first turn reads the stored value at send time and cannot be walked back
 * once it has run.
 */
export async function createAskThread(input: {
  projectId: string;
  title: string;
  agentKind: string;
  model: string | null;
  effort: EffortLevel | null;
  machineId: string | null;
  network: boolean;
}): Promise<AskThread> {
  return invoke<AskThread>("ask_create", {
    input: {
      project_id: input.projectId,
      title: input.title,
      agent_kind: input.agentKind,
      model: input.model,
      effort: input.effort,
      machine_id: input.machineId,
      network: input.network,
    },
  });
}

/** Mirrors `ask_list` — a project's Ask threads, most recently touched first. */
export async function listAskThreads(projectId: string): Promise<AskThread[]> {
  return invoke<AskThread[]>("ask_list", { projectId });
}

/** Mirrors `ask_load` — an Ask thread and its whole transcript. */
export async function loadAskThread(threadId: string): Promise<AskThreadDetail> {
  return invoke<AskThreadDetail>("ask_load", { threadId });
}

/**
 * Mirrors `ask_running` — whether a turn is under way on this thread right
 * now. Asked once when a thread is selected: `ask_turn_status` carries every
 * later change, and a surface that mounted mid-turn has already missed the
 * transition that would have told it.
 */
export async function askTurnRunning(threadId: string): Promise<boolean> {
  return invoke<boolean>("ask_running", { threadId });
}

/** Mirrors `ask_rename`. */
export async function renameAskThread(threadId: string, title: string): Promise<AskThread> {
  return invoke<AskThread>("ask_rename", { threadId, title });
}

/** Mirrors `ask_delete` — deletes the thread and its transcript, via the
 *  declared foreign key. */
export async function deleteAskThread(threadId: string): Promise<void> {
  return invoke<void>("ask_delete", { threadId });
}

/**
 * Mirrors `ask_update_settings`. A key absent from `patch` is left alone; a
 * key set to `null` (`model`, `effort`) clears it. `agent_kind` has no field
 * here — a thread's harness is fixed at creation.
 *
 * `null` is load-bearing rather than a spelling of "no change": it survives to
 * Rust only because `AskSettingsPatch` deserializes those two fields through a
 * `present` helper, since serde's derive collapses `null` and absent alike into
 * "leave alone". Pass a key only when you mean to write it.
 */
export async function updateAskThreadSettings(
  threadId: string,
  patch: { model?: string | null; effort?: EffortLevel | null; network?: boolean },
): Promise<AskThread> {
  return invoke<AskThread>("ask_update_settings", {
    threadId,
    patch: {
      ...("model" in patch ? { model: patch.model } : {}),
      ...("effort" in patch ? { effort: patch.effort } : {}),
      ...("network" in patch ? { network: patch.network } : {}),
    },
  });
}

/**
 * Mirrors `ask_send_turn`. Returns the user's own message, already
 * persisted — the assistant's answer arrives later over the events below.
 */
export async function sendAskTurn(threadId: string, text: string): Promise<AskMessage> {
  return invoke<AskMessage>("ask_send_turn", { threadId, text });
}

// ── The streaming contract (`application/ask/events.rs`) ───────────────────

/** Every `AgentEvent` of a turn, as it arrives. */
export const EVENT_ASK_AGENT_EVENT = "ask_agent_event";
/** A turn's phase: `setting_up`, then `running`, then `idle` or `error`. */
export const EVENT_ASK_TURN_STATUS = "ask_turn_status";
/** The completion signal — a multi-minute turn that ended silently would
 *  force the user to sit and watch it. */
export const EVENT_ASK_TURN_COMPLETED = "ask_turn_completed";

/**
 * Payload of `ask_agent_event`.
 *
 * `event` is the Rust `AgentEvent`, an enum serde-tagged by `kind` and wide
 * enough that mirroring the whole of it here would be a second copy to keep
 * in step for the sake of the variants a consuming surface reads. It stays
 * `unknown` behind guards there, the same convention `lib/discovery.ts` uses.
 */
export interface AskAgentEventPayload {
  thread_id: string;
  event: unknown;
}

/** Mirrors `AskTurnStatus`. */
export interface AskTurnStatusPayload {
  thread_id: string;
  status: string;
  reason: string | null;
}

/** Mirrors `AskTurnCompleted`. */
export interface AskTurnCompletedPayload {
  thread_id: string;
  title: string;
  message_id: string | null;
  ending: "success" | "interrupted" | "failed" | "environmental";
  reason: string | null;
  cost_usd: number;
  tokens: number;
  duration_ms: number;
}
