import { useEffect, useMemo, useRef, useState } from 'react';
import { useTauriEvent } from '../../hooks/useTauriEvent';
import type { Feature, HarnessBaseline, StepExecution } from '../../types';
import { runStatusMeta } from '../../lib/runStatus';
import { useErrorBus } from '../../lib/errorBus';
import { formatError } from '../../lib/errors';
import { formatDuration } from '../../lib/utils';
import { getFeature } from '../../lib/featureSync';
import { listStepsForRun } from '../../lib/featureDetail';
import { readHarnessBaseline, readHarnessEvidence } from '../../lib/harnessVerdict';
import { reconcileSteps } from '../../lib/stepReconcile';
import type { HarnessOverrides } from './useHarnessOverrides';

/**
 * Minimum spacing between two `step_progress`-driven fetches.
 *
 * `step_progress` is not a transition feed: the agent step emits one per
 * streamed text delta, and `PROGRESS_THROTTLE_MS` in
 * `crates/demeteo-core/src/adapters/run_event_log.rs` gates only the
 * *persisted* run-event append — `RunEventRecorder::emit` forwards every
 * event to the UI unchanged. Raising that backend throttle is the wrong
 * lever and has been rejected: the log wants a readable narrative, this view
 * wants smooth telemetry, and they are different jobs. The spacing belongs
 * here.
 */
const PROGRESS_RELOAD_FLOOR_MS = 500;

interface StepProgressPayload {
  feature_id: string;
  step_id: string;
  status: string;
  cost_usd: number | null;
  tokens: number | null;
  wall_clock_secs: number | null;
  cache_read_input_tokens: number | null;
  cache_creation_input_tokens: number | null;
}

interface Deferred {
  promise: Promise<void>;
  resolve: () => void;
}

function deferred(): Deferred {
  let resolve = () => {};
  const promise = new Promise<void>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

/**
 * Serialises reloads: one fetch in flight, one request queued behind it, and
 * `floorMs` between two starts unless a caller asks for the run now.
 *
 * The awaited promise is the load-bearing part. A request never resolves off
 * the fetch already in flight — that fetch may have read the database before
 * the caller's own mutation landed, so joining it would report a stale row as
 * current. It resolves off the *next* run, which is why every `reload()` call
 * site can keep awaiting it and trust what it reads afterwards.
 */
function createReloadCoalescer(run: () => Promise<void>, floorMs: number) {
  let active: Promise<void> | null = null;
  let queued: Deferred | null = null;
  let urgent = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let lastStart = Number.NEGATIVE_INFINITY;

  const launch = () => {
    timer = null;
    const waiters = queued;
    queued = null;
    urgent = false;
    lastStart = Date.now();
    active = (async () => {
      try {
        await run();
      } finally {
        active = null;
        waiters?.resolve();
        arm();
      }
    })();
  };

  const arm = () => {
    if (!queued || active || timer !== null) return;
    const wait = urgent ? 0 : Math.max(0, floorMs - (Date.now() - lastStart));
    if (wait === 0) {
      launch();
      return;
    }
    timer = setTimeout(launch, wait);
  };

  return {
    request(immediate: boolean): Promise<void> {
      queued ??= deferred();
      urgent = urgent || immediate;
      const settled = queued.promise;
      if (urgent && timer !== null) {
        clearTimeout(timer);
        timer = null;
      }
      arm();
      return settled;
    },
    /** Drop the queued run and release its waiters, so an awaiting caller
     *  outlives the unmount rather than hanging on a promise nobody settles. */
    cancel(): void {
      if (timer !== null) {
        clearTimeout(timer);
        timer = null;
      }
      const waiters = queued;
      queued = null;
      urgent = false;
      waiters?.resolve();
    },
  };
}

/**
 * Fold one `step_progress` event into the list already on screen, so telemetry
 * stays live between two coalesced fetches.
 *
 * The event carries the *step* id, and a feature can hold more than one
 * execution row for it, so the newest row wins — the same choice
 * `useRunGraph` makes when it resolves a node to a step. Mid-turn samples are
 * not persisted (`turn.rs` emits without a `step_update`), so the following
 * fetch legitimately answers with the lower, committed numbers.
 */
function patchStepFromProgress(
  steps: StepExecution[],
  payload: StepProgressPayload,
): StepExecution[] {
  const matches = steps.filter((s) => s.step_id === payload.step_id);
  if (matches.length === 0) return steps;
  const target = matches.reduce((a, b) => (b.updated_at >= a.updated_at ? b : a));
  const patched: StepExecution = {
    ...target,
    status: payload.status,
    cost_usd: payload.cost_usd ?? target.cost_usd,
    tokens: payload.tokens ?? target.tokens,
    wall_clock_secs: payload.wall_clock_secs ?? target.wall_clock_secs,
    cache_read_input_tokens: payload.cache_read_input_tokens ?? target.cache_read_input_tokens,
    cache_creation_input_tokens:
      payload.cache_creation_input_tokens ?? target.cache_creation_input_tokens,
  };
  return reconcileSteps(
    steps,
    steps.map((s) => (s === target ? patched : s)),
  );
}

/**
 * The run behind one feature: its steps, its rolled-up telemetry, and the
 * feature row's own fields. Every surface that changes the run — a gate
 * decision, a retry, a remote poll tick — re-reads it through `reload`
 * rather than patching a local copy.
 */
export function useFeatureRun(input: {
  featureId: string;
  projectId: string | undefined;
  initialTitle: string;
  overrides: HarnessOverrides;
}) {
  const { featureId, projectId, initialTitle, overrides } = input;
  const { reportError } = useErrorBus();
  const [steps, setSteps] = useState<StepExecution[]>([]);
  const [featureStatus, setFeatureStatus] = useState('running');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [featureTitle, setFeatureTitle] = useState<string>(initialTitle);
  // The persisted prompt body (migration V27), surfaced in the Initial Prompt
  // panel below. `''` for runs started before the column existed.
  const [featureDescription, setFeatureDescription] = useState<string>('');
  // What this run's validation gates said at the base commit (decision 44,
  // `features.harness_baseline_json`). `null` is "nothing measured" and is
  // rendered as such — never as a pass; see `HarnessGateTable`.
  const [harnessBaseline, setHarnessBaseline] = useState<HarnessBaseline | null>(null);

  const status = useMemo(() => {
    if (featureStatus === 'cancelled') return 'cancelled';
    if (steps.some(s => s.status === 'awaiting_gate')) return 'gated';
    if (steps.some(s => s.status === 'failed')) return 'failed';
    if (steps.some(s => s.status === 'interrupted')) return 'cancelled';
    if (steps.some(s => s.status === 'running')) return 'running';
    if (steps.some(s => s.status === 'verifying')) return 'verifying';
    if (steps.length > 0 && steps.every(s => s.status === 'completed')) return 'completed';
    return featureStatus;
  }, [steps, featureStatus]);
  const statusMeta = runStatusMeta(status);
  const anyStepStarted = steps.some((s) => s.status !== 'pending');
  // What this run's persisted step failures say about the same gates the
  // baseline measured — the *now* half of HB7's table. Read off the last step
  // that reported any, because a rework loop re-runs validate and the earlier
  // attempt describes code that has since changed.
  const harnessEvidence = useMemo(() => readHarnessEvidence(steps), [steps]);

  // Derived rather than stored so the header's spend cannot be anything but
  // the pipeline's: with no setter to hand it, an event handler has no way to
  // put one step's running cost here (audit F19).
  const totals = useMemo(() => {
    let tokens = 0;
    let cost = 0;
    let secs = 0;
    let cacheRead = 0;
    let cacheCreation = 0;
    for (const s of steps) {
      tokens += s.tokens || 0;
      cost += s.cost_usd || 0;
      secs += s.wall_clock_secs || 0;
      cacheRead += s.cache_read_input_tokens || 0;
      cacheCreation += s.cache_creation_input_tokens || 0;
    }
    return { tokens, cost, cacheRead, cacheCreation, duration: formatDuration(secs) };
  }, [steps]);

  const fetchRun = async () => {
    try {
      const list = await listStepsForRun(featureId);
      setSteps((prev) => reconcileSteps(prev, list));

      let f: Feature | null = null;
      try {
        f = await getFeature(featureId);
        if (f) {
          overrides.adoptFeatureModel(f.model);
          if (f.title) {
            setFeatureTitle(f.title);
          }
          if (typeof f.description === 'string') {
            setFeatureDescription(f.description);
          }
          // Through a guard rather than a field read: the column is JSON the
          // engine wrote and a shape this build does not understand must
          // degrade to "no baseline", exactly as `HarnessBaseline::from_column`
          // degrades every decode failure to `None`.
          setHarnessBaseline(readHarnessBaseline(f));
        }
      } catch (err) {
        reportError(err, { kind: "internal" });
      }

      if (f?.status) setFeatureStatus(f.status);

      setError(null);
      setLoading(false);

      const targetProjectId = projectId || f?.project_id;
      if (f && targetProjectId) {
        overrides.probeForFeature({ agentKind: f.agent_kind, projectId: targetProjectId });
      }
    } catch (err) {
      setError(formatError(err));
      setLoading(false);
    }
  };

  const fetchRunRef = useRef(fetchRun);
  fetchRunRef.current = fetchRun;
  const coalescerRef = useRef<ReturnType<typeof createReloadCoalescer> | null>(null);
  if (!coalescerRef.current) {
    coalescerRef.current = createReloadCoalescer(
      () => fetchRunRef.current(),
      PROGRESS_RELOAD_FLOOR_MS,
    );
  }
  const coalescer = coalescerRef.current;

  const reload = () => coalescer.request(true);

  useEffect(() => { reload(); }, [featureId]);

  useEffect(() => () => coalescer.cancel(), [coalescer]);

  useTauriEvent<{ feature_id: string; status: string }>('feature_status_changed', ({ feature_id, status: s }) => {
    if (feature_id === featureId) {
      setFeatureStatus(s);
      reload();
    }
  });

  useTauriEvent<StepProgressPayload>('step_progress', (payload) => {
    if (payload.feature_id !== featureId) return;
    setSteps((prev) => patchStepFromProgress(prev, payload));
    coalescer.request(false);
  });

  return {
    steps,
    status,
    statusMeta,
    setFeatureStatus,
    featureStatus,
    tokens: totals.tokens,
    totalCost: totals.cost,
    cacheReadTokens: totals.cacheRead,
    cacheCreationTokens: totals.cacheCreation,
    duration: totals.duration,
    loading,
    error,
    featureTitle,
    featureDescription,
    harnessBaseline,
    harnessEvidence,
    anyStepStarted,
    reload,
  };
}
