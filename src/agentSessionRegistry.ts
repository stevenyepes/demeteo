/**
 * Frontend agent session registry. Mirrors the backend `AgentRegistry`:
 * tracks per-thread agent turn state (idle / spawning / installing /
 * running / pending_approval / error) and the per-turn `AgentEvent`
 * stream subscription. The actual backend session is created lazily
 * on the first directive; the frontend mirrors that intent.
 *
 * The agent stream itself is delivered via the global Tauri event
 * `agent_event` (with `thread_id` in the payload) — the registry
 * fans events out to subscribers keyed by thread.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AgentEvent,
  ThreadStatus,
  WorkingMemoryEntry,
} from "./types";

export type AgentKind = "opencode" | "hermes" | "noop" | string;

export interface AgentConfigView {
  kind: string;
  enabled: boolean;
  available: boolean;
  install_command: string;
}

interface Listener {
  (event: AgentEvent): void;
}

class AgentSessionRegistry {
  private listenersByThread = new Map<string, Set<Listener>>();
  private statusByThread = new Map<string, ThreadStatus>();
  private unlisten: UnlistenFn | null = null;
  // Tracks the in-flight `listen()` promise so two concurrent
  // `ensureInstalled()` calls (e.g. React 18 StrictMode's
  // mount→unmount→mount effect cycle) don't register two listeners
  // and clobber each other's unlisten handle.
  private installing: Promise<void> | null = null;

  /** Install the global `agent_event` listener once. Idempotent. */
  ensureInstalled(): Promise<void> {
    if (this.unlisten) return Promise.resolve();
    if (this.installing) return this.installing;
    this.installing = listen<{ thread_id: string; event: AgentEvent }>(
      "agent_event",
      (e) => {
        const { thread_id, event } = e.payload;
        if (!thread_id) return;
        const set = this.listenersByThread.get(thread_id);
        if (!set) return;
        // The registry is a thin fan-out. Subscribers (the React tree
        // for that thread) handle the actual rendering.
        for (const l of set) l(event);
      },
    ).then((u) => {
      this.unlisten = u;
      this.installing = null;
    });
    return this.installing;
  }

  async dispose(): Promise<void> {
    // If an install is in flight, wait for it before tearing down so
    // the listener doesn't dangle.
    if (this.installing) {
      try { await this.installing; } catch { /* swallow */ }
    }
    if (this.unlisten) {
      this.unlisten();
      this.unlisten = null;
    }
    this.listenersByThread.clear();
    this.installing = null;
  }

  /** Subscribe to events for a specific thread. Returns the unsubscribe fn. */
  subscribe(threadId: string, l: Listener): () => void {
    let set = this.listenersByThread.get(threadId);
    if (!set) {
      set = new Set();
      this.listenersByThread.set(threadId, set);
    }
    set.add(l);
    return () => {
      const s = this.listenersByThread.get(threadId);
      if (!s) return;
      s.delete(l);
      if (s.size === 0) this.listenersByThread.delete(threadId);
    };
  }

  setStatus(threadId: string, status: ThreadStatus) {
    this.statusByThread.set(threadId, status);
  }

  getStatus(threadId: string): ThreadStatus | undefined {
    return this.statusByThread.get(threadId);
  }
}

export const agentSessionRegistry = new AgentSessionRegistry();

/**
 * High-level API for components. Wraps the Tauri commands and
 * pushes the resulting state into the registry. Components call
 * these directly from event handlers.
 */
export async function loadAgentConfigs(machineId: string): Promise<AgentConfigView[]> {
  return await invoke<AgentConfigView[]>("get_agent_configs", { machineId });
}

export async function setAgentConfigs(
  machineId: string,
  agents: { kind: string; enabled: boolean }[],
): Promise<void> {
  return await invoke<void>("set_agent_configs", { machineId, agents });
}

/**
 * Eagerly spawn the agent. On `NOT_FOUND:binary:install_command`, the
 * caller should present the install_command in a consent modal and
 * call `agentInstallAndStart` on approval. On any other error, the
 * caller should surface the message in the supervisor stream.
 */
export async function agentStart(
  threadId: string,
  agentKind: AgentKind,
): Promise<void> {
  await invoke<void>("agent_start", { threadId, agentKind });
}

export async function agentInstallAndStart(
  threadId: string,
  agentKind: AgentKind,
): Promise<void> {
  await invoke<void>("agent_install_and_start", { threadId, agentKind });
}

/**
 * Send a directive. The agent's per-turn stream is delivered via the
 * global `agent_event` event (subscribed via `agentSessionRegistry`).
 */
export async function agentPrompt(
  threadId: string,
  agentKind: AgentKind,
  text: string,
): Promise<void> {
  await invoke<void>("agent_prompt", { threadId, agentKind, text });
}

export async function agentCancel(threadId: string): Promise<void> {
  await invoke<void>("agent_cancel", { threadId });
}

export async function agentRestart(threadId: string): Promise<void> {
  await invoke<void>("agent_restart", { threadId });
}

export async function loadWorkingMemory(
  threadId: string,
): Promise<WorkingMemoryEntry[]> {
  return await invoke<WorkingMemoryEntry[]>("get_working_memory", { threadId });
}

export async function clearWorkingMemory(threadId: string): Promise<void> {
  await invoke<void>("clear_working_memory", { threadId });
}

/**
 * Update the thread's status optimistically. The backend emits
 * `thread_status_changed` to confirm or correct; see `App.tsx` for
 * the listener.
 */
export async function setThreadStatus(
  threadId: string,
  status: ThreadStatus,
): Promise<void> {
  await invoke<void>("update_thread_status", { id: threadId, status });
}
