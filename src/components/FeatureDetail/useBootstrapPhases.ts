import { useCallback, useMemo, useState } from 'react';
import { useTauriEvent } from '../../hooks/useTauriEvent';
import type { BootstrapProgressPayload } from '../../types';
import { orderBootstrapPhases, type BootstrapPhaseView } from '../BootstrapStepper';

/**
 * Bootstrap sub-step phases (feature-start "phase 0"), keyed by phase id.
 * Fed by the local `bootstrap_progress` Tauri event and, for remote runs, by
 * `bootstrap_progress` entries in the durable run-event log (see
 * `useRemoteRun`). Rendered as an inline stepper above the timeline while the
 * feature is still bootstrapping.
 *
 * The stepper is "phase 0": shown until the first real DAG step leaves
 * `pending` (at which point the status timeline takes over), or while the
 * feature is still `bootstrapping`. A failed bootstrap keeps it visible (no
 * step ever started) so the failing phase + error stay on screen.
 */
export function useBootstrapPhases(input: {
  featureId: string;
  featureStatus: string;
  anyStepStarted: boolean;
}) {
  const { featureId, featureStatus, anyStepStarted } = input;
  const [bootstrapPhases, setBootstrapPhases] = useState<Map<string, BootstrapPhaseView>>(
    () => new Map(),
  );

  const upsertBootstrapPhase = useCallback(
    (p: { phase: string; label?: string; status?: string; detail?: string | null }) => {
      if (!p.phase) return;
      setBootstrapPhases((prev) => {
        const next = new Map(prev);
        next.set(p.phase, {
          id: p.phase,
          label: p.label ?? p.phase,
          status: p.status ?? 'running',
          detail: p.detail ?? null,
        });
        return next;
      });
    },
    [],
  );

  // Local (attached) path: bootstrap sub-steps arrive as Tauri events.
  useTauriEvent<BootstrapProgressPayload>('bootstrap_progress', (p) => {
    if (p.feature_id && p.feature_id !== featureId) return;
    upsertBootstrapPhase(p);
  });

  const orderedBootstrapPhases = useMemo<BootstrapPhaseView[]>(() => {
    const ordered = orderBootstrapPhases(bootstrapPhases);
    if (ordered.length === 0 && featureStatus === 'bootstrapping') {
      // Fresh launch, before the first event lands — show a running
      // placeholder so the panel isn't momentarily blank.
      return [{ id: 'preparing', label: 'Loading project & workflow', status: 'running', detail: null }];
    }
    return ordered;
  }, [bootstrapPhases, featureStatus]);

  return {
    bootstrapPhases,
    upsertBootstrapPhase,
    orderedBootstrapPhases,
    showBootstrap: !anyStepStarted && orderedBootstrapPhases.length > 0,
  };
}
