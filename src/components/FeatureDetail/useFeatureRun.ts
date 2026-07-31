import { useEffect, useMemo, useState } from 'react';
import { useTauriEvent } from '../../hooks/useTauriEvent';
import type { Feature, HarnessBaseline, StepExecution } from '../../types';
import { runStatusMeta } from '../../lib/runStatus';
import { useErrorBus } from '../../lib/errorBus';
import { formatError } from '../../lib/errors';
import { formatDuration } from '../../lib/utils';
import { getFeature } from '../../lib/featureSync';
import { listStepsForRun } from '../../lib/featureDetail';
import { readHarnessBaseline, readHarnessEvidence } from '../../lib/harnessVerdict';
import type { HarnessOverrides } from './useHarnessOverrides';

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
  const [tokens, setTokens] = useState<number>(0);
  const [totalCost, setTotalCost] = useState<number>(0);
  const [cacheReadTokens, setCacheReadTokens] = useState<number>(0);
  const [cacheCreationTokens, setCacheCreationTokens] = useState<number>(0);
  const [duration, setDuration] = useState('0s');
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

  const reload = async () => {
    try {
      const list = await listStepsForRun(featureId);
      setSteps(list);

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

      let totalTokens = 0;
      let totalCost = 0;
      let totalSecs = 0;
      let totalCacheRead = 0;
      let totalCacheCreation = 0;
      for (const s of list) {
        totalTokens += s.tokens || 0;
        totalCost += s.cost_usd || 0;
        totalSecs += s.wall_clock_secs || 0;
        totalCacheRead += s.cache_read_input_tokens || 0;
        totalCacheCreation += s.cache_creation_input_tokens || 0;
      }
      setTokens(totalTokens);
      setTotalCost(totalCost);
      setCacheReadTokens(totalCacheRead);
      setCacheCreationTokens(totalCacheCreation);
      setDuration(formatDuration(totalSecs));
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

  useEffect(() => { reload(); }, [featureId]);

  useTauriEvent<{ feature_id: string; status: string }>('feature_status_changed', ({ feature_id, status: s }) => {
    if (feature_id === featureId) {
      setFeatureStatus(s);
      reload();
    }
  });

  useTauriEvent<{ feature_id: string; step_id: string; status: string; cost_usd: number | null; tokens: number | null; wall_clock_secs: number | null; cache_read_input_tokens: number | null; cache_creation_input_tokens: number | null }>('step_progress', (payload) => {
    if (payload.feature_id !== featureId) return;
    // Live-update the total cost so the header chip reflects the
    // current step's running spend without waiting for a full
    // feature reload.
    if (typeof payload.cost_usd === 'number') {
      setTotalCost(payload.cost_usd);
    }
    reload();
  });

  return {
    steps,
    status,
    statusMeta,
    setFeatureStatus,
    featureStatus,
    tokens,
    totalCost,
    cacheReadTokens,
    cacheCreationTokens,
    duration,
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
