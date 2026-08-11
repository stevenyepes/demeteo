import React, { useState, useEffect, useCallback } from 'react';
import { RemoteRunMirror, StepExecution } from '../types';
import { Check, ArrowRight, X, ShieldAlert, Terminal, Sparkles, AlertTriangle } from 'lucide-react';
import { ArtifactViewer } from './ArtifactViewer';
import { GateArtifactPicker } from './GateArtifactPicker';
import { useArtifactSelection } from './FeatureDetail/useArtifactSelection';
import { useErrorBus } from '../lib/errorBus';
import {
  decideGate,
  getStepExecution,
  isBlockingError,
  findActivePredecessor,
  listStepsForRun,
  type GateBlocker,
} from '../lib/features';
import { decideRemoteGate, remoteRunForFeature } from '../lib/remoteRuns';

interface GateViewProps {
  stepExecutionId: string;
  onDecisionSubmitted: () => void;
  onClose: () => void;
}

export const GateView: React.FC<GateViewProps> = ({
  stepExecutionId,
  onDecisionSubmitted,
  onClose,
}) => {
  const { reportError } = useErrorBus();
  const [stepExec, setStepExec] = useState<StepExecution | null>(null);
  const [feedback, setFeedback] = useState('');
  const [isRedirecting, setIsRedirecting] = useState(false);
  const [, setLoading] = useState(true);
  // First non-terminal predecessor of the gate step (if any). When
  // non-null, both the Approve and Redirect buttons are disabled and
  // a rose-bordered banner is rendered above them so the user is not
  // lured into approving a gate whose predecessor agent step is still
  // running.
  const [blockedBy, setBlockedBy] = useState<GateBlocker | null>(null);
  // Mirror row when this gate belongs to a detached run. The local
  // `gate_decide` is a no-op for such a feature (the laptop only holds a
  // read-only shadow — the run's real DB and driver live on the runner),
  // so the decision has to go over the tunnel instead. `null` = ordinary
  // local run.
  const [remoteRun, setRemoteRun] = useState<RemoteRunMirror | null>(null);
  // Every step of the run, kept alongside `blockedBy` so the artifact
  // picker below has the full predecessor set to choose from. The
  // blocking-predecessor probe below reads this same list independently —
  // neither derivation may be made to depend on the other.
  const [allSteps, setAllSteps] = useState<StepExecution[]>([]);

  const { selectedArtifactPath, selectedArtifactVersion, openArtifact } =
    useArtifactSelection(allSteps);

  const loadGateData = useCallback(async () => {
    try {
      const execDetails = await getStepExecution(stepExecutionId);
      setStepExec(execDetails);
      setRemoteRun(await remoteRunForFeature(execDetails.feature_id));
      // Re-probe the predecessor set on every load. The parent's
      // `feature_status_changed` event triggers a remount via the
      // navigation effect, so the banner clears within one tick of
      // the blocking predecessor transitioning to `completed`.
      const all = await listStepsForRun(execDetails.feature_id);
      setAllSteps(all);
      const blocker = findActivePredecessor(all, execDetails);
      setBlockedBy(
        blocker
          ? {
              id: blocker.id,
              step_id: blocker.step_id,
              status: blocker.status,
              step_index: blocker.step_index,
            }
          : null,
      );
    } catch (err) {
      reportError(err);
    } finally {
      setLoading(false);
    }
  }, [stepExecutionId, reportError]);

  useEffect(() => {
    loadGateData();
  }, [loadGateData]);

  // Default the picker to today's behavior — the immediate predecessor's
  // first artifact — the first time both the step list and the gate step
  // itself have loaded. Once the reviewer picks a row, `selectedArtifactPath`
  // is no longer null and this effect never fires again for this mount.
  useEffect(() => {
    if (selectedArtifactPath !== null) return;
    if (!stepExec || allSteps.length === 0) return;
    const defaultPath = stepExec.artifact_paths?.length
      ? stepExec.artifact_paths[0]
      : stepExec.artifact_path;
    if (defaultPath) {
      openArtifact(defaultPath, stepExec.step_id);
    }
  }, [stepExec, allSteps, selectedArtifactPath, openArtifact]);

  const submitDecision = async (decision: 'approve' | 'redirect' | 'cancel') => {
    // Defence-in-depth: also short-circuit in the modal so a double-click
    // doesn't fire a redundant IPC after the parent re-renders.
    if (blockedBy && decision !== 'cancel') return;
    setLoading(true);
    try {
      const gateFeedback = decision === 'redirect' ? feedback : null;
      if (remoteRun) {
        // The runner's `decide_gate` RPC keys the gate by its
        // step_execution_id and delegates to the same `GatePresenter`
        // call as the local path, so `approve | redirect | cancel` mean
        // exactly what they mean here.
        await decideRemoteGate({
          machineId: remoteRun.machine_id,
          runId: remoteRun.run_id,
          gateId: stepExecutionId,
          decision,
          feedback: gateFeedback,
        });
      } else {
        await decideGate({ stepExecutionId, decision, feedback: gateFeedback });
      }
      onDecisionSubmitted();
    } catch (err) {
      // Blocking-predecessor errors are already surfaced by the
      // modal banner above, so skip the toast (would be redundant).
      // All other errors propagate through the error bus.
      if (!isBlockingError(err)) {
        reportError(err);
      }
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-md p-4">
      {/* Modal Card */}
      {/* Wider than the app's other modals because this one has to show an
          artifact — a task list or a diff at 42rem wraps every line. */}
      <div className="w-full max-w-4xl bg-[#0d0f14] border border-violet-500/30 rounded-2xl shadow-[0_0_50px_rgba(139,92,246,0.15)] overflow-hidden flex flex-col font-sans max-h-[85vh]">
        {/* Modal Header */}
        <div className="p-6 border-b border-white/5 bg-white/[0.01] flex items-center justify-between">
          <div className="flex items-center gap-3">
            <span className="p-2 rounded-lg bg-amber-500/10 text-amber-400 border border-amber-500/20 shadow-[0_0_10px_rgba(245,158,11,0.1)]">
              <ShieldAlert className="w-5 h-5" />
            </span>
            <div>
              <h2 className="text-lg font-bold font-heading text-white tracking-wide">Manual Approval Gate</h2>
              <p className="text-xs text-slate-400">Review findings and authorize the next pipeline stage</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 bg-white/5 hover:bg-white/10 rounded-lg text-slate-400 hover:text-white transition"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Modal Content */}
        <div className="p-6 flex-1 min-h-0 overflow-y-auto space-y-6">
          <div className="p-4 rounded-lg bg-white/[0.01] border border-white/5 text-sm text-slate-400 leading-relaxed space-y-3">
            <div className="text-white font-semibold flex items-center gap-1.5 text-xs uppercase tracking-wider text-slate-300">
              <Terminal className="w-3.5 h-3.5" /> Pipeline context
            </div>
            <p>
              The multi-agent workflow is currently **paused** at the step **{stepExec?.step_id ? stepExec.step_id.replace("s-", "").replace(/-/g, " ") : 'Gate Step'}**.
              Review the artifact generated below.
            </p>
          </div>

          {/* Artifact Preview */}
          <div className="space-y-2 flex flex-col">
            <div className="text-white font-semibold flex items-center gap-1.5 text-xs uppercase tracking-wider text-slate-300 shrink-0">
              <Sparkles className="w-3.5 h-3.5 text-violet-400" /> Artifact Output
            </div>
            {/* A definite height, not a min-height: this box sits inside a
                scrollable body, so `flex-1` here resolves against nothing and
                the editor below inherits a height of zero — which is how the
                artifact rendered as an empty black panel. `vh` keeps it
                proportional to the window instead of a magic pixel count. */}
            {stepExec && (
              <GateArtifactPicker
                steps={allSteps}
                gateStepIndex={stepExec.step_index}
                selectedArtifactPath={selectedArtifactPath}
                onSelectArtifact={openArtifact}
              />
            )}
            <div className="flex h-[46vh] min-h-[240px] flex-col p-4 rounded-lg border border-white/5 bg-[#050608] overflow-hidden">
              {selectedArtifactPath ? (
                <ArtifactViewer artifactPath={selectedArtifactPath} contentVersion={selectedArtifactVersion} />
              ) : (
                <div className="text-slate-500 font-mono text-xs italic flex items-center justify-center h-full">
                  No artifact outputs saved for this gate step.
                </div>
              )}
            </div>
          </div>

          {/* Redirect / Loop feedback */}
          {isRedirecting && (
            <div className="space-y-2 animate-fade-in">
              <label className="block text-xs uppercase tracking-wider text-slate-400 font-bold">
                Redirection Feedback / Instructions
              </label>
              <textarea
                value={feedback}
                onChange={(e) => setFeedback(e.target.value)}
                placeholder="Instruct the agent on what failed, what they need to fix, or which step to retry."
                rows={3}
                className="w-full bg-[#050608] border border-white/10 focus:border-violet-500 rounded-lg p-3 text-xs text-white focus:outline-none transition resize-none leading-relaxed font-sans"
              />
            </div>
          )}
        </div>

        {/* Blocking banner: surfaced when an earlier step is still
            running. The banner sits above the action buttons so the
            user cannot miss the precondition violation. "Abort
            feature" intentionally remains enabled — aborting is a
            separate intent from approving. */}
        {blockedBy && (
          <div
            data-testid="gate-blocked-banner"
            className="mx-6 mb-3 p-3 rounded-lg border border-rose-500/30 bg-rose-500/10 text-xs text-rose-300 flex items-start gap-2"
            title={`Cannot decide this gate while '${blockedBy.step_id}' is ${blockedBy.status}`}
          >
            <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0" />
            <span className="leading-relaxed">
              <span className="font-bold uppercase tracking-wider">Decision blocked.</span>{' '}
              Step <span className="font-mono font-semibold text-rose-200">{blockedBy.step_id}</span> is still{' '}
              <span className="font-mono">{blockedBy.status}</span>. Wait for it to finish before deciding this gate.
            </span>
          </div>
        )}

        {/* Modal Footer Actions */}
        <div className="p-6 border-t border-white/5 bg-white/[0.01] flex items-center justify-between">
          <button
            onClick={() => submitDecision('cancel')}
            className="px-4 py-2 border border-rose-500/20 hover:border-rose-500/50 bg-rose-500/10 hover:bg-rose-500/20 text-rose-400 hover:text-white rounded-lg text-xs font-bold transition duration-300"
          >
            Abort feature
          </button>

          <div className="flex gap-2">
            {isRedirecting ? (
              <>
                <button
                  onClick={() => setIsRedirecting(false)}
                  className="px-4 py-2 bg-white/5 hover:bg-white/10 rounded-lg text-xs font-semibold transition"
                >
                  Back
                </button>
                <button
                  onClick={() => submitDecision('redirect')}
                  disabled={blockedBy !== null}
                  title={
                    blockedBy
                      ? `Cannot redirect while '${blockedBy.step_id}' is ${blockedBy.status}`
                      : 'Send the redirect feedback to the agent'
                  }
                  className="flex items-center gap-1.5 px-4 py-2 bg-violet-600 hover:bg-violet-500 hover:shadow-[0_0_15px_rgba(139,92,246,0.4)] disabled:bg-violet-900/40 disabled:hover:bg-violet-900/40 disabled:cursor-not-allowed disabled:shadow-none rounded-lg text-xs font-bold text-white transition duration-300"
                >
                  Send Redirect <ArrowRight className="w-3.5 h-3.5" />
                </button>
              </>
            ) : (
              <>
                <button
                  onClick={() => setIsRedirecting(true)}
                  disabled={blockedBy !== null}
                  title={
                    blockedBy
                      ? `Cannot redirect while '${blockedBy.step_id}' is ${blockedBy.status}`
                      : 'Switch into redirect / loop mode'
                  }
                  className="px-4 py-2 border border-violet-500/30 bg-violet-500/10 hover:bg-violet-500/20 text-violet-400 hover:text-white disabled:border-violet-900/30 disabled:bg-violet-900/20 disabled:text-violet-700 disabled:cursor-not-allowed rounded-lg text-xs font-bold transition duration-300"
                >
                  Redirect / Loop
                </button>
                <button
                  onClick={() => submitDecision('approve')}
                  disabled={blockedBy !== null}
                  title={
                    blockedBy
                      ? `Cannot approve while '${blockedBy.step_id}' is ${blockedBy.status}`
                      : 'Approve this gate and let the pipeline continue'
                  }
                  className="flex items-center gap-1.5 px-5 py-2 bg-emerald-600 hover:bg-emerald-500 hover:shadow-[0_0_20px_rgba(16,185,129,0.5)] disabled:bg-emerald-900/40 disabled:hover:bg-emerald-900/40 disabled:cursor-not-allowed disabled:shadow-none rounded-lg text-xs font-bold text-white transition duration-300 shadow-[0_0_15px_rgba(16,185,129,0.3)]"
                >
                  Approve step <Check className="w-3.5 h-3.5" />
                </button>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};
