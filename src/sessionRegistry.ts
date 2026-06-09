/**
 * Frontend session registry. Owns the *intent* to keep a session alive
 * independently of which view is currently rendered. The actual backend
 * SSH session is created lazily on first attach, and only destroyed when
 * `destroySession` is called explicitly (tab close, machine delete) or
 * the backend reaps it on idle/EOF.
 */
import { invoke, Channel } from "@tauri-apps/api/core";

export type SessionStatus = "starting" | "ready" | "ended" | "error";

export interface SessionRecord {
  sessionId: string | null;
  status: SessionStatus;
  errorDetail?: string;
}

type Listener = () => void;

class SessionRegistry {
  private records = new Map<string, SessionRecord>();
  private listeners = new Set<Listener>();

  key(machineId: string, tabId: string) {
    return `${machineId}::${tabId}`;
  }

  get(machineId: string, tabId: string): SessionRecord {
    const k = this.key(machineId, tabId);
    let rec = this.records.get(k);
    if (!rec) {
      rec = { sessionId: null, status: "starting" };
      this.records.set(k, rec);
    }
    return rec;
  }

  update(machineId: string, tabId: string, patch: Partial<SessionRecord>) {
    const k = this.key(machineId, tabId);
    const cur = this.get(machineId, tabId);
    this.records.set(k, { ...cur, ...patch });
    this.notify();
  }

  forgetTab(machineId: string, tabId: string) {
    this.records.delete(this.key(machineId, tabId));
    this.notify();
  }

  forgetMachine(machineId: string) {
    for (const k of [...this.records.keys()]) {
      if (k.startsWith(`${machineId}::`)) this.records.delete(k);
    }
    this.notify();
  }

  /** All live backend session ids belonging to a given machine */
  liveSessionIds(machineId: string): string[] {
    const out: string[] = [];
    for (const [k, rec] of this.records.entries()) {
      if (k.startsWith(`${machineId}::`) && rec.sessionId) out.push(rec.sessionId);
    }
    return out;
  }

  /** Find the tab id (for a given machine) that owns a particular backend session id */
  findTabBySessionId(machineId: string, sessionId: string): string | null {
    for (const [k, rec] of this.records.entries()) {
      if (!k.startsWith(`${machineId}::`)) continue;
      if (rec.sessionId === sessionId) return k.split("::")[1];
    }
    return null;
  }

  subscribe(l: Listener): () => void {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  }

  private notify() {
    this.listeners.forEach((l) => l());
  }
}

export const sessionRegistry = new SessionRegistry();

/**
 * Start a new backend SSH session (or return the existing one if the
 * registry already has a sessionId for this tab — which means a previous
 * SSHTerminal instance had it open and the backend session is still alive).
 */
export async function ensureSession(
  machineId: string,
  tabId: string,
  onData: (bytes: number[]) => void,
): Promise<string> {
  const existing = sessionRegistry.get(machineId, tabId);
  if (existing.sessionId) {
    // Reattach to the still-alive backend session with a fresh data channel
    const channel = new Channel<number[]>();
    channel.onmessage = onData;
    await invoke("attach_terminal_session", {
      sessionId: existing.sessionId,
      tauriChannel: channel,
    });
    sessionRegistry.update(machineId, tabId, { status: "ready" });
    return existing.sessionId;
  }

  const channel = new Channel<number[]>();
  channel.onmessage = onData;
  try {
    const sessId = await invoke<string>("start_terminal_session", {
      machineId,
      tauriChannel: channel,
    });
    sessionRegistry.update(machineId, tabId, {
      sessionId: sessId,
      status: "ready",
    });
    return sessId;
  } catch (e) {
    sessionRegistry.update(machineId, tabId, {
      status: "error",
      errorDetail: String(e),
    });
    throw e;
  }
}

/** Detach the current frontend channel without destroying the backend session. */
export async function detachSession(machineId: string, tabId: string) {
  const rec = sessionRegistry.get(machineId, tabId);
  if (!rec.sessionId) return;
  try {
    await invoke("detach_terminal_session", { sessionId: rec.sessionId });
  } catch (e) {
    console.error("detach failed", e);
  }
}

/** Destroy the backend session and clear the registry entry. */
export async function destroySession(machineId: string, tabId: string) {
  const rec = sessionRegistry.get(machineId, tabId);
  if (rec.sessionId) {
    try {
      await invoke("close_terminal_session", { sessionId: rec.sessionId });
    } catch (e) {
      console.error("close failed", e);
    }
  }
  // Wipe saved scrollback so a freshly-opened tab doesn't pick up old content
  try {
    localStorage.removeItem(`demeteo.termbuf.${machineId}.${tabId}`);
  } catch {
    // ignore
  }
  sessionRegistry.forgetTab(machineId, tabId);
}
