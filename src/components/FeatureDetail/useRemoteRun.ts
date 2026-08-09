import { useCallback, useEffect, useState } from 'react';
import type { RemoteRunMirror, RunEvent } from '../../types';
import { TERMINAL_STATUSES } from '../../lib/runStatus';
import { listMachines, remoteRefreshRun, remoteRunForFeature } from '../../lib/featureDetail';

type BootstrapPhasePayload = { phase: string; label?: string; status?: string; detail?: string | null };

function isBootstrapPhasePayload(value: unknown): value is BootstrapPhasePayload {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { phase?: unknown }).phase === 'string'
  );
}

const POLL_BASE_MS = 3_000;
const POLL_CEILING_MS = 48_000;

/**
 * How long to wait before the next tick given the run of failures behind it.
 * A tick costs a tunnel round trip plus the two IPC calls in `reload()`, so a
 * tunnel that is gone must not be charged the same rate as a runner that is
 * merely busy; the ceiling keeps a recovered tunnel from taking minutes to be
 * noticed.
 */
function pollDelayMs(consecutiveFailures: number): number {
  if (consecutiveFailures < 1) return POLL_BASE_MS;
  return Math.min(POLL_BASE_MS * 2 ** consecutiveFailures, POLL_CEILING_MS);
}

/**
 * Remote (shadow) run live refresh (docs/REMOTE_EXECUTION.md M6.4). A feature
 * that ran on a remote machine receives none of the local `step_progress` /
 * `feature_status_changed` Tauri events — those fire on the runner, not this
 * laptop — so without this its timeline would freeze at whatever the last
 * reconcile captured. While such a run is non-terminal we poll its runner
 * (`remote_refresh_run`, which re-hydrates the shadow) and re-read the
 * mirrored steps, driving the exact same timeline UI a local run gets from
 * its events. `remoteRun` is `null` for a locally-run feature (not in the
 * mirror) — the poll never starts.
 *
 * The poll is gated on `document.hidden` and backs off on failure
 * (docs/UI_REDESIGN_PLAN.md §4.8): nobody reads a timeline behind a hidden
 * window, and an unattended laptop left on a running feature would otherwise
 * spend hours of tunnel traffic and battery on frames no one sees. Becoming
 * visible ticks at once rather than joining the schedule, because the reason to
 * come back to the window is to see the current state.
 */
export function useRemoteRun(input: {
  featureId: string;
  reload: () => void;
  upsertBootstrapPhase: (p: BootstrapPhasePayload) => void;
}) {
  const { featureId, reload, upsertBootstrapPhase } = input;
  const [remoteRun, setRemoteRun] = useState<RemoteRunMirror | null>(null);
  // Display name for `remoteRun.machine_id` — resolved lazily (only
  // when the feature turns out to be a remote run) from the same
  // machines list every other view uses.
  const [remoteMachineName, setRemoteMachineName] = useState<string | null>(null);
  // Remote runs don't emit local `run_event` Tauri pushes — the runner does —
  // so their unified feed comes from the `remote_stream_events` poll that the
  // Activity strip already tails; we capture the same batch here (P2.6) so the
  // node panel's Overview raw feed works for both transports from one shape.
  const [remoteRunEvents, setRemoteRunEvents] = useState<RunEvent[]>([]);

  useEffect(() => {
    let cancelled = false;
    remoteRunForFeature(featureId)
      .then((r) => { if (!cancelled) setRemoteRun(r); })
      .catch(() => { if (!cancelled) setRemoteRun(null); });
    return () => { cancelled = true; };
  }, [featureId]);

  useEffect(() => {
    if (!remoteRun) {
      setRemoteMachineName(null);
      return;
    }
    let cancelled = false;
    listMachines()
      .then((machines) => {
        if (cancelled) return;
        setRemoteMachineName(
          machines.find((m) => m.id === remoteRun.machine_id)?.name ?? null,
        );
      })
      .catch(() => { /* the raw machine id is an acceptable fallback */ });
    return () => { cancelled = true; };
  }, [remoteRun?.machine_id]);

  // Immediate re-sync after a user action on the remote run (gate
  // decided from the Activity section) — the poll would catch up on its
  // own schedule, but the decision should reflect instantly.
  const refreshRemoteRun = async () => {
    if (!remoteRun) return;
    try {
      const updated = await remoteRefreshRun({
        machineId: remoteRun.machine_id,
        runId: remoteRun.run_id,
      });
      if (updated) setRemoteRun(updated);
      reload();
    } catch {
      // Transient tunnel hiccup — the next poll tick retries.
    }
  };

  useEffect(() => {
    if (!remoteRun) return;
    const { machine_id: machineId, run_id: runId } = remoteRun;
    // A finished run's shadow can't change further — pull once so the
    // terminal state's steps/artifacts are current, then stop polling.
    if (TERMINAL_STATUSES.includes(remoteRun.status)) {
      reload();
      return;
    }
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let consecutiveFailures = 0;
    let inFlight = false;

    const clearPending = () => {
      if (timer !== undefined) clearTimeout(timer);
      timer = undefined;
    };
    const schedule = (delayMs: number) => {
      clearPending();
      timer = setTimeout(() => { timer = undefined; void poll(); }, delayMs);
    };

    const poll = async () => {
      // A visibility flip can land while a tick is awaiting the tunnel; that
      // tick reschedules itself on settling, so a second one here would only
      // double the traffic.
      if (cancelled || inFlight) return;
      inFlight = true;
      try {
        const updated = await remoteRefreshRun({ machineId, runId });
        if (cancelled) return;
        consecutiveFailures = 0;
        if (updated) setRemoteRun(updated);
        reload();
      } catch {
        // Transient tunnel hiccup — the next tick retries. Nothing to
        // surface: the shadow keeps showing the last good state.
        if (cancelled) return;
        consecutiveFailures += 1;
      } finally {
        inFlight = false;
      }
      if (document.hidden) return;
      schedule(pollDelayMs(consecutiveFailures));
    };

    const onVisibilityChange = () => {
      clearPending();
      if (!document.hidden) void poll();
    };

    if (!document.hidden) schedule(pollDelayMs(0));
    document.addEventListener('visibilitychange', onVisibilityChange);
    return () => {
      cancelled = true;
      clearPending();
      document.removeEventListener('visibilitychange', onVisibilityChange);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [remoteRun?.machine_id, remoteRun?.run_id, remoteRun?.status]);

  // Remote (detached) path: bootstrap sub-steps live in the run-event log.
  // `RunEventTimeline` polls it; we tap the same batch via `onEvents` (no
  // second poll), lift `bootstrap_progress` entries into the phase map, and
  // retain the raw feed so the node panel's Overview (P2.6) shows the same
  // unified log a local run gets from its Tauri `run_event` pushes.
  const handleRunEvents = useCallback(
    (evts: RunEvent[]) => {
      for (const e of evts) {
        if (e.kind !== 'bootstrap_progress' || !e.payload_json) continue;
        try {
          const p: unknown = JSON.parse(e.payload_json);
          if (isBootstrapPhasePayload(p)) upsertBootstrapPhase(p);
        } catch {
          /* malformed payload — skip */
        }
      }
      // Retain the raw feed, de-duped by offset (the poll can re-deliver a
      // batch across a reconnect) and capped to a recent window.
      setRemoteRunEvents((prev) => {
        const seen = new Set(prev.map((e) => e.offset));
        const fresh = evts.filter((e) => !seen.has(e.offset));
        if (fresh.length === 0) return prev;
        const next = [...prev, ...fresh];
        return next.length > 500 ? next.slice(next.length - 500) : next;
      });
    },
    [upsertBootstrapPhase],
  );

  return { remoteRun, remoteMachineName, remoteRunEvents, refreshRemoteRun, handleRunEvents };
}
