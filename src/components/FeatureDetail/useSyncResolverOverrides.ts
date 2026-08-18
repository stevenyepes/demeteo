import { useEffect, useState } from 'react';
import { getSyncResolver } from '../../lib/featureSync';
import { useErrorBus } from '../../lib/errorBus';
import type { SyncResolverView } from '../../types';
import { useHarnessOverrides, type HarnessOverrides } from './useHarnessOverrides';

export interface SyncResolverSelection {
  overrides: HarnessOverrides;
  /** What an untouched picker resolves to, or `null` until the read lands. */
  inherited: SyncResolverView | null;
}

/**
 * The conflict banner's own harness selection, and the identity it inherits.
 *
 * A second `useHarnessOverrides` deliberately: the one `FeatureDetailView`
 * hands the step cards is what a retry re-pins, so sharing it would make a
 * harness chosen for one merge conflict the harness the next retry runs the
 * whole step under.
 *
 * The inherited identity is **read from the backend**, not assembled here. Two
 * reasons, and the first is the one that bites: the resolver chain puts the
 * project's conflict-resolver setting above the feature's own row, so the run's
 * harness — the only one this component ever knew — names the wrong agent for
 * any project that has configured one, and the effort ladder derived from it
 * can offer a level the harness that really runs would drop at `clamp_for`.
 * The second is that spelling the precedence again in TypeScript is a copy that
 * goes stale silently.
 *
 * Probing is deferred until that read lands for a reason no reading of
 * `probeForFeature` shows: it is a once-only latch (a second call with a better
 * answer early-returns), so probing with a placeholder harness pins the wrong
 * model list for the life of the view.
 */
export function useSyncResolverOverrides(input: {
  featureId: string;
  projectId: string | undefined;
  conflicted: boolean;
}): SyncResolverSelection {
  const { featureId, projectId, conflicted } = input;
  const { reportError } = useErrorBus();
  const overrides = useHarnessOverrides();
  const [inherited, setInherited] = useState<SyncResolverView | null>(null);
  const probeForFeature = overrides.probeForFeature;

  useEffect(() => {
    if (!conflicted) return;
    let cancelled = false;
    (async () => {
      try {
        const resolver = await getSyncResolver(featureId);
        if (!cancelled) setInherited(resolver);
      } catch (err) {
        if (!cancelled) reportError(err, { kind: 'internal' });
      }
    })();
    return () => { cancelled = true; };
  }, [conflicted, featureId, reportError]);

  useEffect(() => {
    if (!inherited || !projectId) return;
    probeForFeature({ agentKind: inherited.agent_kind, projectId });
  }, [inherited, projectId, probeForFeature]);

  return { overrides, inherited };
}
