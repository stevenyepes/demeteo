import { useCallback, useRef, useState } from 'react';
import { confirm as confirmDialog, message as messageDialog } from '@tauri-apps/plugin-dialog';
import type { RemoteRunMirror } from '../../types';
import { formatError } from '../../lib/errors';
import {
  retryStep,
  replayFromStep,
  remoteRetryStep,
  remoteReplayStep,
  isBlockingError,
} from '../../lib/features';
import { cancelFeature, remoteCancelRun } from '../../lib/featureDetail';
import type { HarnessOverrides } from './useHarnessOverrides';

export interface ReplayTarget {
  id: string;
  name: string;
  downstreamCount: number;
}

/**
 * Stopping, retrying and replaying the run — the actions that rewind or end
 * it, each of which has to pick the local or the runner-side RPC.
 *
 * `handleRetryStep` and `handleStopStep` reach every memoized `StepCard`, so
 * they never change identity. That rules out depending on the inputs: `reload`
 * and `refreshRemoteRun` are rebuilt by their own hooks on every render, so a
 * dependency array naming them would stabilize nothing. They are read at call
 * time through a ref instead, the shape `useStepSelection` uses for `navigate`.
 */
export function useRerunActions(input: {
  featureId: string;
  remoteRun: RemoteRunMirror | null;
  refreshRemoteRun: () => void;
  reload: () => void;
  setFeatureStatus: (status: string) => void;
  overrides: HarnessOverrides;
}) {
  const latest = useRef(input);
  latest.current = input;
  const [replayTarget, setReplayTarget] = useState<ReplayTarget | null>(null);
  // The replay cone (target + descendants) ringed on the canvas while the
  // replay confirm modal is open (P2.4). Null when no panel-initiated replay.
  const [replayPreviewNodes, setReplayPreviewNodes] = useState<Set<string> | null>(null);

  const startReplay = useCallback((target: ReplayTarget, previewNodes: Set<string> | null) => {
    setReplayPreviewNodes(previewNodes);
    setReplayTarget(target);
  }, []);

  // Dismiss the replay modal and drop the canvas highlight together.
  const closeReplay = useCallback(() => {
    setReplayTarget(null);
    setReplayPreviewNodes(null);
  }, []);

  /** Stop the run behind this feature. Cancellation is feature-wide on
   * both paths — the local `feature_cancel` signals the driver, which
   * unwinds whatever step it is on — so "Stop Step" and "Cancel Feature"
   * differ only in their wording. A detached run has no local driver to
   * signal (the laptop holds a read-only shadow), so it must be cancelled
   * on the runner over the tunnel instead; the local call would find no
   * cancel sender for the feature and return `Ok` having done nothing. */
  const cancelRun = useCallback(async (failureTitle: string) => {
    const { featureId, remoteRun, refreshRemoteRun, setFeatureStatus } = latest.current;
    try {
      if (remoteRun) {
        await remoteCancelRun({
          machineId: remoteRun.machine_id,
          runId: remoteRun.run_id,
        });
        refreshRemoteRun();
      } else {
        await cancelFeature(featureId);
      }
      setFeatureStatus('cancelled');
      // feature_status_changed event will fire and call reload reactively
    } catch (err) {
      await messageDialog(formatError(err), { title: failureTitle, kind: 'error' });
    }
  }, []);

  const handleCancelFeature = useCallback(async () => {
    const ok = await confirmDialog('Are you sure you want to cancel the execution of this feature?', {
      title: 'Cancel Feature',
      kind: 'warning',
      okLabel: 'Cancel Feature',
      cancelLabel: 'Keep Running',
    });
    if (!ok) return;
    await cancelRun('Cancel Failed');
  }, [cancelRun]);

  const handleStopStep = useCallback(async () => {
    const ok = await confirmDialog('Are you sure you want to stop the execution of this step?', {
      title: 'Stop Step',
      kind: 'warning',
      okLabel: 'Stop Step',
      cancelLabel: 'Keep Running',
    });
    if (!ok) return;
    await cancelRun('Stop Failed');
  }, [cancelRun]);

  const handleRetryStep = useCallback(async (stepExecutionId: string) => {
    const { remoteRun, refreshRemoteRun, reload, overrides } = latest.current;
    try {
      const modelParam = overrides.selectedModel || null;
      const agentParam = overrides.selectedAgent || null;
      const effortParam = overrides.selectedEffort || null;
      if (remoteRun) {
        // A detached run is retried on the runner: this machine has no
        // driver for it and no worktree to replay into. The shadow mirrors
        // the runner's step ids verbatim, so the local id is the right one
        // to name. The command re-injects the PAT and re-opens the run, so
        // the retried pipeline still pushes and opens its PR at the end.
        await remoteRetryStep({
          machineId: remoteRun.machine_id,
          runId: remoteRun.run_id,
          stepExecutionId,
          model: modelParam,
          agentKind: agentParam,
          effort: effortParam,
        });
        refreshRemoteRun();
      } else {
        await retryStep({ stepExecutionId, newModel: modelParam, newAgent: agentParam, newEffort: effortParam });
      }
      reload();
    } catch (err) {
      // Blocking-predecessor errors are surfaced as warnings rather
      // than errors — the user did nothing wrong, the UI was stale.
      const isBlocking = isBlockingError(err);
      await messageDialog(formatError(err), {
        title: isBlocking ? 'Retry Blocked' : 'Retry Failed',
        kind: isBlocking ? 'warning' : 'error',
      });
    }
  }, []);

  const handleReplayFromStep = useCallback(async () => {
    if (!replayTarget) return;
    const { remoteRun, refreshRemoteRun, reload, overrides } = latest.current;
    try {
      const modelParam = overrides.selectedModel || null;
      const agentParam = overrides.selectedAgent || null;
      const effortParam = overrides.selectedEffort || null;
      if (remoteRun) {
        // Deliberately the *replay* RPC, not `remoteRetryStep`. They are
        // not one rewind wearing two labels: retry refuses a step that
        // isn't failed/interrupted/pending, and a replay target is normally
        // completed. Sharing the retry call made remote replay always fail
        // with "Cannot retry a step in 'completed' status".
        await remoteReplayStep({
          machineId: remoteRun.machine_id,
          runId: remoteRun.run_id,
          stepExecutionId: replayTarget.id,
          model: modelParam,
          agentKind: agentParam,
          effort: effortParam,
        });
        refreshRemoteRun();
      } else {
        await replayFromStep({ stepExecutionId: replayTarget.id, newModel: modelParam, newAgent: agentParam, newEffort: effortParam });
      }
      setReplayTarget(null);
      setReplayPreviewNodes(null);
      reload();
    } catch (err) {
      await messageDialog(formatError(err), { title: 'Replay Failed', kind: 'error' });
    }
  }, [replayTarget]);

  return {
    replayTarget,
    replayPreviewNodes,
    startReplay,
    closeReplay,
    handleCancelFeature,
    handleStopStep,
    handleRetryStep,
    handleReplayFromStep,
  };
}
