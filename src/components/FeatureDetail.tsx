import React, { useState, useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { confirm as confirmDialog, message as messageDialog } from '@tauri-apps/plugin-dialog';
import { StepExecution } from '../types';
import { getAgentModels } from '../lib/agentModels';
import { ShieldAlert, CheckCircle, RefreshCw, XCircle, ArrowRight, Hourglass, Cpu, X, GitPullRequest, RotateCcw } from 'lucide-react';
import { ArtifactViewer } from './ArtifactViewer';

interface FeatureDetailProps {
  featureId: string;
  projectId?: string;
  title: string;
  onDecideGate: (stepExecId: string) => void;
  onBack: () => void;
}

const humanizeStepId = (id: string) => {
  return id
    .replace(/^s-/, '')
    .split('-')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
};

export const FeatureDetail: React.FC<FeatureDetailProps> = ({
  featureId,
  projectId,
  title,
  onDecideGate,
  onBack,
}) => {
  const [steps, setSteps] = useState<StepExecution[]>([]);
  const [status, setStatus] = useState('running');
  const [cost, setCost] = useState(0.0);
  const [duration, setDuration] = useState('0s');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedArtifactPath, setSelectedArtifactPath] = useState<string | null>(null);
  const [selectedStepTitle, setSelectedStepTitle] = useState<string | null>(null);
  const [activeStreamId, setActiveStreamId] = useState<string | null>(null);
  const [streamContent, setStreamContent] = useState<Record<string, string>>({});
  const [availableModels, setAvailableModels] = useState<Array<{ value: string; name: string }>>([]);
  const [selectedModel, setSelectedModel] = useState<string>('');
  const [isLoadingModels, setIsLoadingModels] = useState(false);
  const [replayTarget, setReplayTarget] = useState<{ id: string; name: string; downstreamCount: number } | null>(null);

  useEffect(() => {
    loadFeatureData();

    // Subscribe to Tauri events
    let active = true;
    const cleanups: Array<() => void> = [];

    const setupListeners = async () => {
      try {
        const unlistenStatus = await listen<{ feature_id: string; status: string }>(
          'feature_status_changed',
          (event) => {
            if (event.payload.feature_id === featureId) {
              setStatus(event.payload.status);
              loadFeatureData();
            }
          }
        );
        if (!active) {
          unlistenStatus();
        } else {
          cleanups.push(unlistenStatus);
        }

        const unlistenProgress = await listen<{
          feature_id: string;
          step_id: string;
          status: string;
          cost_usd: number | null;
          wall_clock_secs: number | null;
        }>('step_progress', (event) => {
          if (event.payload.feature_id === featureId) {
            loadFeatureData();
          }
        });
        if (!active) {
          unlistenProgress();
        } else {
          cleanups.push(unlistenProgress);
        }

        const unlistenGate = await listen<{ feature_id: string; step_execution_id: string }>(
          'gate_required',
          (event) => {
            if (event.payload.feature_id === featureId) {
              onDecideGate(event.payload.step_execution_id);
            }
          }
        );
        if (!active) {
          unlistenGate();
        } else {
          cleanups.push(unlistenGate);
        }

        const unlistenStream = await listen<{ feature_id: string; step_execution_id: string; content: string }>(
          'agent_stream',
          (event) => {
            if (event.payload.feature_id === featureId) {
              setStreamContent((prev) => ({
                ...prev,
                [event.payload.step_execution_id]: (prev[event.payload.step_execution_id] || '') + event.payload.content
              }));
            }
          }
        );
        if (!active) {
          unlistenStream();
        } else {
          cleanups.push(unlistenStream);
        }
      } catch (err) {
        console.error('Failed to setup Tauri event listeners:', err);
      }
    };

    setupListeners();

    return () => {
      active = false;
      cleanups.forEach((unlisten) => unlisten());
    };
  }, [featureId]);
  const loadFeatureData = async () => {
    try {
      const list = await invoke<StepExecution[]>('step_list_for_run', { featureId });
      setSteps(list);

      let f: any = null;
      try {
        f = await invoke('feature_get', { featureId });
        if (f && selectedModel === '') {
          setSelectedModel(f.model || '');
        }
      } catch (err) {
        console.error('Failed to fetch feature detail:', err);
      }

      // Compute telemetry
      let totalCost = 0.0;
      let totalSecs = 0;
      let isGated = false;
      let hasFailed = false;
      let isRunning = false;
      let hasInterrupted = false;

      for (const s of list) {
        totalCost += s.cost_usd || 0.0;
        totalSecs += s.wall_clock_secs || 0;
        if (s.status === 'awaiting_gate') isGated = true;
        if (s.status === 'failed') hasFailed = true;
        if (s.status === 'running') isRunning = true;
        if (s.status === 'interrupted') hasInterrupted = true;
      }

      setCost(totalCost);
      setDuration(`${totalSecs}s`);

      if (f?.status === 'cancelled') setStatus('cancelled');
      else if (isGated) setStatus('gated');
      else if (hasFailed) setStatus('failed');
      else if (hasInterrupted) setStatus('cancelled');
      else if (isRunning) setStatus('running');
      else if (list.every(s => s.status === 'completed')) setStatus('completed');

      setError(null);
      setLoading(false);

      const targetProjectId = projectId || f?.project_id;
      if (f && targetProjectId && availableModels.length === 0 && !isLoadingModels) {
        setIsLoadingModels(true);
        (async () => {
          try {
            const project = await invoke<any>('get_project_by_id', { projectId: targetProjectId });
            const machineId = project?.remote_host || 'local';
            const agentKind = f.agent_kind || 'opencode';
            const models = await getAgentModels(machineId, agentKind);
            setAvailableModels(models as Array<{ value: string; name: string }>);
          } catch (err) {
            console.warn('Failed to fetch available models:', err);
          } finally {
            setIsLoadingModels(false);
          }
        })();
      }
    } catch (err: any) {
      setError(err.toString());
      setLoading(false);
    }
  };

  const handleCancelFeature = async () => {
    const ok = await confirmDialog('Are you sure you want to cancel the execution of this feature?', {
      title: 'Cancel Feature',
      kind: 'warning',
      okLabel: 'Cancel Feature',
      cancelLabel: 'Keep Running',
    });
    if (!ok) return;
    try {
      await invoke('feature_cancel', { featureId });
      setStatus('cancelled');
      // The backend processes cancellation asynchronously; poll until
      // the feature status flips so the UI doesn't revert to "running".
      for (let i = 0; i < 20; i++) {
        await new Promise(r => setTimeout(r, 200));
        const f2: any = await invoke('feature_get', { featureId }).catch(() => null);
        if (f2?.status === 'cancelled' || f2?.status === 'failed') break;
        await loadFeatureData();
      }
    } catch (err: any) {
      await messageDialog(err.toString(), { title: 'Cancel Failed', kind: 'error' });
    }
  };

  const handleStopStep = async () => {
    const ok = await confirmDialog('Are you sure you want to stop the execution of this step?', {
      title: 'Stop Step',
      kind: 'warning',
      okLabel: 'Stop Step',
      cancelLabel: 'Keep Running',
    });
    if (!ok) return;
    try {
      await invoke('feature_cancel', { featureId });
      setStatus('cancelled');
      for (let i = 0; i < 20; i++) {
        await new Promise(r => setTimeout(r, 200));
        const f2: any = await invoke('feature_get', { featureId }).catch(() => null);
        if (f2?.status === 'cancelled' || f2?.status === 'failed') break;
        await loadFeatureData();
      }
    } catch (err: any) {
      await messageDialog(err.toString(), { title: 'Stop Failed', kind: 'error' });
    }
  };

  const handleRetryStep = async (stepExecutionId: string) => {
    try {
      const modelParam = selectedModel || null;
      await invoke('step_retry', { stepExecutionId, newModel: modelParam });
      loadFeatureData();
    } catch (err: any) {
      await messageDialog(err.toString(), { title: 'Retry Failed', kind: 'error' });
    }
  };

  const handleReplayFromStep = async () => {
    if (!replayTarget) return;
    try {
      const modelParam = selectedModel || null;
      await invoke('replay_from_step', { stepExecutionId: replayTarget.id, newModel: modelParam });
      setReplayTarget(null);
      loadFeatureData();
    } catch (err: any) {
      await messageDialog(err.toString(), { title: 'Replay Failed', kind: 'error' });
    }
  };

  /** Publish the feature branch as a PR/MR via the project's
   *  connected provider (R6). The backend is idempotent: re-publish
   *  on an already-published feature returns the existing URL
   *  instead of creating a duplicate. */
  const [publishing, setPublishing] = useState(false);
  const handlePublish = async (draft: boolean) => {
    if (!projectId) {
      await messageDialog('No project is associated with this feature.', {
        title: 'Cannot publish',
        kind: 'error',
      });
      return;
    }
    setPublishing(true);
    try {
      const result: any = await invoke('publish_mr', {
        projectId,
        featureId,
        draft,
      });
      const url = result?.url ?? '(unknown)';
      const state = result?.state ?? 'open';
      await messageDialog(
        `MR/PR opened (state: ${state}).\n\n${url}`,
        { title: 'Published', kind: 'info' },
      );
      loadFeatureData();
    } catch (err: any) {
      await messageDialog(err.toString(), { title: 'Publish failed', kind: 'error' });
    } finally {
      setPublishing(false);
    }
  };

  /** Apply the project's `feature_lifecycle` policy (R6 decision 26).
   *  `archive` → soft-delete; `auto_delete` → git branch -D +
   *  soft-delete; `keep` → no-op. */
  const handleCleanup = async () => {
    try {
      const result: any = await invoke('feature_cleanup', { featureId });
      await messageDialog(
        `Cleanup (${result.policy}): ${result.action}`,
        { title: 'Lifecycle applied', kind: 'info' },
      );
      loadFeatureData();
    } catch (err: any) {
      await messageDialog(err.toString(), { title: 'Cleanup failed', kind: 'error' });
    }
  };

  return (
    <div className="h-full w-full bg-[#08090c] text-slate-100 flex flex-col font-sans">
      {/* Header telemetry panel */}
      <div className="p-6 border-b border-white/5 bg-[#0d0f14]/80 flex items-center justify-between backdrop-blur-md">
        <div className="space-y-1">
          <div className="flex items-center gap-3">
            <button
              onClick={onBack}
              className="text-xs px-2.5 py-1 bg-white/5 hover:bg-white/10 rounded text-slate-400 hover:text-white transition uppercase font-bold"
            >
              Back
            </button>
            <h1 className="text-xl font-bold font-display text-white tracking-wide">Feature Pipeline</h1>
            <span
              className={`text-xs px-2.5 py-0.5 rounded-full font-bold uppercase border tracking-wider ${
                status === 'running'
                  ? 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20 animate-pulse'
                  : status === 'gated'
                  ? 'bg-amber-500/10 text-amber-400 border-amber-500/20 shadow-[0_0_10px_rgba(245,158,11,0.2)]'
                  : status === 'completed'
                  ? 'bg-cyan-500/10 text-cyan-400 border-cyan-500/20'
                  : 'bg-rose-500/10 text-rose-400 border-rose-500/20'
              }`}
            >
              {status}
            </span>
          </div>
          <p className="text-xs text-slate-400">ID: {featureId}</p>
        </div>

        <div className="flex items-center gap-6">
          <div className="text-right">
            <div className="text-[10px] text-slate-500 uppercase font-bold">Elapsed Duration</div>
            <div className="text-lg font-bold font-mono text-white">{duration}</div>
          </div>
          <div className="text-right">
            <div className="text-[10px] text-slate-500 uppercase font-bold">Pipeline Cost</div>
            <div className="text-lg font-bold font-mono text-cyan-400">${cost.toFixed(3)}</div>
          </div>
          {status === 'running' && (
            <button
              onClick={handleCancelFeature}
              className="px-4 py-2 bg-rose-600/20 hover:bg-rose-600 border border-rose-500/30 text-rose-400 hover:text-white rounded-lg text-xs font-bold transition duration-300"
            >
              Cancel Feature
            </button>
          )}
          {(status === 'completed' || status === 'failed' || status === 'cancelled') && (
            <>
              <button
                onClick={() => handlePublish(false)}
                disabled={publishing}
                className="px-4 py-2 bg-emerald-600/20 hover:bg-emerald-600 border border-emerald-500/30 text-emerald-400 hover:text-white rounded-lg text-xs font-bold transition duration-300 disabled:opacity-40 flex items-center gap-1.5"
                title="Open a PR/MR for review (R6)"
              >
                {publishing ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <GitPullRequest className="w-3.5 h-3.5" />}
                Publish MR
              </button>
              <button
                onClick={handleCleanup}
                className="px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 rounded-lg text-xs font-bold transition duration-300"
                title="Apply the project's feature_lifecycle (archive / keep / auto_delete)"
              >
                Cleanup
              </button>
            </>
          )}
        </div>
      </div>

      {/* Feature Objective panel */}
      <div className="p-6 bg-[#08090c] border-b border-white/5">
        <div className="max-w-4xl mx-auto flex flex-col gap-2">
          <div className="text-xs text-violet-400 font-bold uppercase tracking-widest flex items-center gap-2">
            Initial Prompt
          </div>
          <div className="p-4 bg-white/[0.02] rounded-xl border border-white/5 text-sm text-slate-300 font-mono whitespace-pre-wrap leading-relaxed shadow-inner">
            {title}
          </div>
        </div>
      </div>

      {loading ? (
        <div className="flex-1 flex items-center justify-center">
          <RefreshCw className="w-8 h-8 text-violet-500 animate-spin" />
        </div>
      ) : error ? (
        <div className="flex-1 p-8 text-rose-400 flex items-center gap-2">
          <ShieldAlert className="w-5 h-5" />
          <span>Error loading details: {error}</span>
        </div>
      ) : (
        <div className="flex-1 flex flex-row overflow-hidden w-full h-full">
          {/* Left Column: Timeline */}
          <div className={`overflow-y-auto p-8 transition-all duration-500 ${
            selectedArtifactPath ? 'w-[40%] border-r border-white/5 bg-[#08090c]/40' : 'w-full max-w-3xl mx-auto'
          }`}>
            <div className="relative border-l border-white/5 ml-4 pl-8 space-y-6">
              {steps.map((step, idx) => {
                let icon = <Hourglass className="w-4 h-4 text-slate-500 animate-pulse" />;
                let statusBg = 'border-white/5 bg-white/[0.01]';

                if (step.status === 'completed') {
                  icon = <CheckCircle className="w-4 h-4 text-emerald-400" />;
                  statusBg = 'border-emerald-500/20 bg-emerald-950/5';
                } else if (step.status === 'failed') {
                  icon = <XCircle className="w-4 h-4 text-rose-400" />;
                  statusBg = 'border-rose-500/20 bg-rose-950/5';
                } else if (step.status === 'running') {
                  icon = <Cpu className="w-4 h-4 text-cyan-400 animate-spin" />;
                  statusBg = 'border-cyan-500/30 bg-cyan-950/10 shadow-[0_0_15px_rgba(6,182,212,0.05)]';
                } else if (step.status === 'awaiting_gate') {
                  icon = <ShieldAlert className="w-4 h-4 text-amber-400 animate-bounce" />;
                  statusBg = 'border-amber-500/40 bg-amber-950/10 shadow-[0_0_15px_rgba(245,158,11,0.08)]';
                }

                return (
                  <div key={step.id} className="relative group">
                    {/* Connector node circle */}
                    <span className="absolute -left-[41px] top-1.5 flex items-center justify-center w-6 h-6 rounded-full bg-[#08090c] border border-white/10">
                      <span className="text-[10px] text-slate-400 font-bold">{idx + 1}</span>
                    </span>

                    <div className={`p-5 rounded-xl border transition-all duration-300 ${statusBg}`}>
                      <div className="flex items-center justify-between">
                        <div className="flex items-center gap-3">
                          {icon}
                          <span className="font-semibold text-white tracking-wide text-sm">{humanizeStepId(step.step_id)}</span>
                          <span className="text-[9px] px-2 py-0.5 rounded bg-white/5 text-slate-400 font-mono">
                            {step.step_kind}
                          </span>
                          <button
                            onClick={() => setReplayTarget({
                              id: step.id,
                              name: humanizeStepId(step.step_id),
                              downstreamCount: steps.length - idx - 1,
                            })}
                            className="opacity-0 group-hover:opacity-100 transition-opacity duration-200 flex items-center gap-1 px-2 py-1 rounded text-[10px] text-cyan-400/60 hover:text-cyan-300 hover:bg-cyan-500/10 font-bold uppercase tracking-wider"
                            title="Replay from this step"
                          >
                            <RotateCcw className="w-3 h-3" /> Replay
                          </button>
                        </div>

                        <div className="flex items-center gap-4 text-xs font-mono">
                          {typeof step.cost_usd === 'number' && <span className="text-cyan-400">${step.cost_usd.toFixed(3)}</span>}
                          {step.wall_clock_secs !== null && <span className="text-slate-400">{step.wall_clock_secs}s</span>}
                        </div>
                      </div>

                      {step.status === 'awaiting_gate' && (
                        <div className="mt-4 p-4 rounded bg-amber-500/5 border border-amber-500/20 flex justify-between items-center animate-pulse">
                          <div className="text-xs text-amber-400 font-semibold uppercase tracking-wide">
                            Pipeline paused. Awaiting manual review.
                          </div>
                          <button
                            onClick={() => onDecideGate(step.id)}
                            className="flex items-center gap-1.5 px-3 py-1.5 bg-amber-500 hover:bg-amber-600 rounded text-xs font-bold text-black transition shadow-[0_0_10px_rgba(245,158,11,0.4)]"
                          >
                            Decide Gate <ArrowRight className="w-3 h-3" />
                          </button>
                        </div>
                      )}

                      {(step.status === 'failed' || step.status === 'interrupted') && step.error_message && (
                        <div className="mt-3 p-3 rounded bg-rose-500/5 border border-rose-500/20 text-xs text-rose-400 font-mono">
                          {step.error_message}
                        </div>
                      )}

                      {(step.status === 'failed' || step.status === 'interrupted') && (
                        <div className="mt-4 p-4 rounded bg-rose-500/5 border border-rose-500/20 flex flex-col gap-3">
                          <div className="flex justify-between items-center">
                            <div className="text-xs text-rose-400 font-semibold uppercase tracking-wide">
                              Step failed. You can change model and retry.
                            </div>
                            <button
                              onClick={() => handleRetryStep(step.id)}
                              className="flex items-center gap-1.5 px-3 py-1.5 bg-rose-600 hover:bg-rose-500 text-white rounded text-xs font-bold transition shadow-[0_0_10px_rgba(239,68,68,0.4)]"
                            >
                              <RefreshCw className="w-3 h-3 animate-pulse" /> Retry Step
                            </button>
                          </div>

                          {isLoadingModels ? (
                            <div className="text-[10px] text-slate-500 font-mono animate-pulse">
                              Probing available models...
                            </div>
                          ) : availableModels.length > 0 ? (
                            <div className="flex items-center gap-3 bg-black/20 p-2.5 rounded border border-white/5">
                              <label className="text-[10px] uppercase font-bold text-slate-400 shrink-0 font-mono">Run with Model:</label>
                              <select
                                value={selectedModel}
                                onChange={(e) => setSelectedModel(e.target.value)}
                                className="flex-1 bg-[#0d0f14] border border-white/10 rounded px-2.5 py-1.5 text-xs text-slate-200 outline-none focus:border-violet-500/50 font-mono cursor-pointer"
                              >
                                <option value="">Default (From Workflow)</option>
                                {availableModels.map((m) => (
                                  <option key={m.value} value={m.value}>
                                    {m.name}
                                  </option>
                                ))}
                              </select>
                            </div>
                          ) : null}
                        </div>
                      )}

                      {(step.artifact_paths?.length ? step.artifact_paths : step.artifact_path ? [step.artifact_path] : []).map((path) => (
                        <button
                          key={path}
                          onClick={() => {
                            setSelectedArtifactPath(path);
                            setSelectedStepTitle(step.step_id);
                          }}
                          className={`mt-3 w-full text-left text-xs font-mono p-3 rounded border flex items-center justify-between transition duration-300 ${
                            selectedArtifactPath === path
                              ? 'bg-violet-950/20 border-violet-500/30 text-violet-300 shadow-[0_0_15px_rgba(139,92,246,0.1)]'
                              : 'bg-[#050608] border-white/[0.02] text-slate-400 hover:border-white/10 hover:bg-white/[0.02] hover:text-white cursor-pointer'
                          }`}
                        >
                          <span className="truncate">Artifact: {path.split('/').pop()}</span>
                          <span className="text-[9px] uppercase font-bold text-slate-500 shrink-0">View Output</span>
                        </button>
                      ))}

                      {step.status === 'running' && (
                        <div className="mt-3 flex gap-2">
                          <button
                            onClick={() => setActiveStreamId(activeStreamId === step.id ? null : step.id)}
                            className="flex-1 text-left text-xs font-mono p-3 rounded border flex items-center justify-between transition duration-300 bg-[#050608] border-white/[0.02] text-cyan-400 hover:border-cyan-500/30 hover:bg-cyan-950/20 cursor-pointer"
                          >
                            <span className="truncate flex items-center gap-2">
                              <Cpu className="w-3 h-3 animate-spin" />
                              View Agent Reasoning
                            </span>
                            <span className="text-[9px] uppercase font-bold text-cyan-500 shrink-0">
                              {activeStreamId === step.id ? 'Hide Stream' : 'Live Stream'}
                            </span>
                          </button>

                          <button
                            onClick={handleStopStep}
                            className="px-4 py-2.5 bg-rose-600/20 hover:bg-rose-600 border border-rose-500/30 text-rose-400 hover:text-white rounded-lg text-xs font-bold transition duration-300 flex items-center gap-1.5 shrink-0"
                            title="Stop this step execution"
                          >
                            <XCircle className="w-3.5 h-3.5" />
                            Stop Step
                          </button>
                        </div>
                      )}

                      {activeStreamId === step.id && (
                        <div className="mt-2 p-3 rounded-lg bg-[#020304] border border-cyan-500/20 max-h-64 overflow-y-auto font-mono text-[11px] shadow-inner flex flex-col-reverse">
                          <pre className="text-cyan-300/80 whitespace-pre-wrap break-words">
                            {streamContent[step.id] || 'Waiting for agent output...'}
                          </pre>
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>

          {/* Right Column: Artifact Viewer panel */}
          <div 
            className={`h-full overflow-hidden border-l border-white/5 bg-[#0d0f14]/60 backdrop-blur-xl flex flex-col transition-all duration-500 ${
              selectedArtifactPath ? 'w-[60%] opacity-100 translate-x-0' : 'w-0 opacity-0 translate-x-[50px] pointer-events-none'
            }`}
          >
            {selectedArtifactPath && (
              <div className="flex-1 flex flex-col p-6 overflow-hidden h-full">
                {/* Header */}
                <div className="flex items-center justify-between border-b border-white/5 pb-4 mb-4 shrink-0">
                  <div>
                    <h3 className="text-sm font-bold text-white font-display uppercase tracking-wider">
                      Artifact Preview
                    </h3>
                    <p className="text-[10px] text-slate-500 font-mono mt-0.5 truncate">
                      {selectedStepTitle ? humanizeStepId(selectedStepTitle) : ''}
                    </p>
                  </div>
                  <button
                    onClick={() => {
                      setSelectedArtifactPath(null);
                      setSelectedStepTitle(null);
                    }}
                    className="p-1.5 bg-white/5 hover:bg-white/10 rounded-lg text-slate-400 hover:text-white transition duration-150"
                  >
                    <X className="w-4 h-4" />
                  </button>
                </div>

                {/* Content */}
                <div className="flex-1 flex flex-col overflow-hidden">
                  <ArtifactViewer artifactPath={selectedArtifactPath} />
                </div>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Replay from step confirmation modal */}
      {replayTarget && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div className="bg-[#0d0f14] border border-white/10 rounded-2xl p-6 max-w-md w-full mx-4 shadow-[0_0_40px_rgba(0,0,0,0.5)]">
            <div className="flex items-center gap-3 mb-4">
              <div className="w-8 h-8 rounded-full bg-cyan-500/10 border border-cyan-500/20 flex items-center justify-center">
                <RotateCcw className="w-4 h-4 text-cyan-400" />
              </div>
              <div>
                <h3 className="text-sm font-bold text-white font-display tracking-wide">
                  Replay from "{replayTarget.name}"
                </h3>
                <p className="text-[10px] text-slate-500 font-mono mt-0.5">
                  {replayTarget.downstreamCount > 0
                    ? `${replayTarget.downstreamCount} downstream step${replayTarget.downstreamCount > 1 ? 's' : ''} will be re-executed`
                    : 'Only this step will be re-executed'}
                </p>
              </div>
            </div>

            <p className="text-xs text-slate-400 mb-5 leading-relaxed">
              Current artifacts for the affected steps will be replaced.
              {status === 'running' && ' The current execution will be cancelled.'}
            </p>

            {availableModels.length > 0 && (
              <div className="flex items-center gap-3 bg-black/20 p-2.5 rounded border border-white/5 mb-5">
                <label className="text-[10px] uppercase font-bold text-slate-400 shrink-0 font-mono">Model:</label>
                <select
                  value={selectedModel}
                  onChange={(e) => setSelectedModel(e.target.value)}
                  className="flex-1 bg-[#0d0f14] border border-white/10 rounded px-2.5 py-1.5 text-xs text-slate-200 outline-none focus:border-violet-500/50 font-mono cursor-pointer"
                >
                  <option value="">Default (From Workflow)</option>
                  {availableModels.map((m) => (
                    <option key={m.value} value={m.value}>{m.name}</option>
                  ))}
                </select>
              </div>
            )}

            <div className="flex justify-end gap-2">
              <button
                onClick={() => setReplayTarget(null)}
                className="px-4 py-2 bg-white/5 hover:bg-white/10 rounded-lg text-xs font-semibold transition"
              >
                Cancel
              </button>
              <button
                onClick={handleReplayFromStep}
                className="flex items-center gap-1.5 px-4 py-2 bg-emerald-600 hover:bg-emerald-500 hover:shadow-[0_0_20px_rgba(16,185,129,0.5)] rounded-lg text-xs font-bold text-white transition duration-300 shadow-[0_0_15px_rgba(16,185,129,0.3)]"
              >
                <RotateCcw className="w-3 h-3" /> Replay
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
