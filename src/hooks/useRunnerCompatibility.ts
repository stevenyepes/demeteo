import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import {
  cachedRunnerCompatibility,
  fetchRunnerCompatibility,
  shouldProbe,
} from '../lib/runnerCompatibility';
import type { RunnerCompatibilityReport } from '../types';

export interface RunnerCompatibilityState {
  /** `null` while the first probe is in flight, for a machine with nothing to
   *  probe, and after a failed probe — a failure is not a verdict, and the
   *  submit gate re-checks the runner regardless. */
  report: RunnerCompatibilityReport | null;
  loading: boolean;
  /** Re-probe past the cache, e.g. after the runner was reinstalled. */
  refresh: () => void;
}

interface Probe {
  machineId: string;
  report: RunnerCompatibilityReport | null;
  settled: boolean;
}

/**
 * Whether `machineId`'s runner is this app's build, for a launch surface to
 * show before the user submits.
 *
 * Every answer is tagged with the machine it was asked for and read only while
 * that machine is still selected, so a slow probe of a machine the user has
 * already switched away from never lands on the one they switched to.
 */
export function useRunnerCompatibility(machineId: string): RunnerCompatibilityState {
  const probes = shouldProbe(machineId);
  const [probe, setProbe] = useState<Probe | null>(null);
  const [refreshes, setRefreshes] = useState(0);
  const lastRefreshes = useRef(refreshes);

  useEffect(() => {
    const force = refreshes !== lastRefreshes.current;
    lastRefreshes.current = refreshes;
    if (!shouldProbe(machineId)) return;

    let current = true;
    setProbe((prev) => ({
      machineId,
      report: prev?.machineId === machineId ? prev.report : cachedRunnerCompatibility(machineId),
      settled: false,
    }));
    fetchRunnerCompatibility(machineId, { force }).then(
      (report) => {
        if (current) setProbe({ machineId, report, settled: true });
      },
      () => {
        if (current) setProbe({ machineId, report: null, settled: true });
      },
    );
    return () => {
      current = false;
    };
  }, [machineId, refreshes]);

  const refresh = useCallback(() => setRefreshes((n) => n + 1), []);

  const mine = probes && probe?.machineId === machineId ? probe : null;
  const report = !probes ? null : mine ? mine.report : cachedRunnerCompatibility(machineId);
  const loading = probes && !mine?.settled;

  return useMemo(() => ({ report, loading, refresh }), [report, loading, refresh]);
}
