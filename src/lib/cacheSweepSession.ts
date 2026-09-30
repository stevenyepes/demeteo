import { listen } from "@tauri-apps/api/event";

import { formatError } from "./errors";
import { sweepFeatureCaches, type CacheSweepReport } from "./featureDetail";

/** Mirrors `DomainEvent::CacheSweepProgress`, emitted by every sweep. */
export interface CacheSweepProgress {
  dry_run: boolean;
  /** `null` once the sweep has finished. */
  project_id: string | null;
  project_index: number;
  project_count: number;
  deleting: string | null;
}

export type CacheSweepBusy = "scanning" | "reclaiming";

/**
 * Lives at module scope, not in the Storage tab: a sweep outlasts the tab
 * that asked for it (minutes on a remote machine), and one held in component
 * state was thrown away on unmount — then re-asked for on the next visit,
 * queueing a second full sweep behind the first.
 */
export interface CacheSweepSession {
  busy: CacheSweepBusy | null;
  report: CacheSweepReport | null;
  error: string | null;
  finishedAt: number | null;
  /** The sweep the backend is in right now — this one or the background one. */
  running: CacheSweepProgress | null;
}

const IDLE: CacheSweepSession = {
  busy: null,
  report: null,
  error: null,
  finishedAt: null,
  running: null,
};

let session = IDLE;
const listeners = new Set<() => void>();
let listening = false;

function update(patch: Partial<CacheSweepSession>) {
  session = { ...session, ...patch };
  for (const listener of listeners) listener();
}

export function subscribeCacheSweep(onChange: () => void): () => void {
  listeners.add(onChange);
  if (!listening) {
    listening = true;
    listen<CacheSweepProgress>("cache_sweep_progress", (e) => {
      update({ running: e.payload.project_id === null ? null : e.payload });
    }).catch((err) => {
      listening = false;
      console.error("[cacheSweepSession] failed to subscribe to cache_sweep_progress", err);
    });
  }
  return () => listeners.delete(onChange);
}

export function readCacheSweep(): CacheSweepSession {
  return session;
}

/** Start a sweep unless one of ours is already in flight; never queues a second. */
export async function startCacheSweep(dryRun: boolean): Promise<void> {
  if (session.busy) return;
  update({ busy: dryRun ? "scanning" : "reclaiming", error: null });
  try {
    const report = await sweepFeatureCaches(dryRun);
    update({ busy: null, report, running: null, finishedAt: Date.now() });
  } catch (err) {
    update({ busy: null, error: formatError(err), running: null });
  }
}

/** Tests only: module state otherwise leaks from one render to the next. */
export function resetCacheSweepSession() {
  session = IDLE;
  listeners.clear();
  listening = false;
}

export interface SweepStatusText {
  title: string;
  detail: string | null;
}

/** What the Storage tab says while a sweep is in flight, or `null` when none is. */
export function describeSweepStatus(
  s: CacheSweepSession,
  projectNames: Readonly<Record<string, string>>,
): SweepStatusText | null {
  const run = s.running;
  if (!s.busy && !run) return null;
  if (!run) {
    return {
      title: s.busy === "reclaiming" ? "Reclaiming…" : "Scanning every project…",
      detail: "Waiting for the first project — or for a background sweep that is already running.",
    };
  }
  const name = (run.project_id && projectNames[run.project_id]) || run.project_id;
  const where = `project ${run.project_index + 1} of ${run.project_count}: ${name}`;
  const detail = run.deleting ? `Deleting ${run.deleting}` : null;
  if (run.dry_run) return { title: `Scanning ${where}`, detail };
  if (s.busy === "reclaiming") return { title: `Reclaiming ${where}`, detail };
  return {
    title: `${s.busy ? "Waiting for the background sweep, which is reclaiming" : "A background sweep is reclaiming"} ${where}`,
    detail,
  };
}
