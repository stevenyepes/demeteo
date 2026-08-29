import { invoke } from "@tauri-apps/api/core";
import type { AskThread, AskThreadDetail, EffortLevel } from "../types";

/**
 * Typed IPC wrappers for Ask — the commands in `src-tauri/src/commands/ask.rs`.
 *
 * Storage and lifecycle only: create, project list, load, rename, delete.
 * No turn execution belongs here — that is `ask-turn-loop`'s to add.
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
 */
export async function createAskThread(input: {
  projectId: string;
  title: string;
  agentKind: string;
  model: string | null;
  effort: EffortLevel | null;
  machineId: string | null;
}): Promise<AskThread> {
  return invoke<AskThread>("ask_create", {
    input: {
      project_id: input.projectId,
      title: input.title,
      agent_kind: input.agentKind,
      model: input.model,
      effort: input.effort,
      machine_id: input.machineId,
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

/** Mirrors `ask_rename`. */
export async function renameAskThread(threadId: string, title: string): Promise<AskThread> {
  return invoke<AskThread>("ask_rename", { threadId, title });
}

/** Mirrors `ask_delete` — deletes the thread and its transcript, via the
 *  declared foreign key. */
export async function deleteAskThread(threadId: string): Promise<void> {
  return invoke<void>("ask_delete", { threadId });
}
