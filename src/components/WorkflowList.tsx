import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { WorkflowWithSteps } from '../types';
import { Play, Pencil, Download, Trash, RefreshCw, Plus, Cpu, GitBranch, ShieldAlert, Clock } from 'lucide-react';
import { useErrorBus } from '../lib/errorBus';
import { formatError } from '../lib/errors';

interface WorkflowListProps {
  onEdit: (id: string) => void;
  onNew: () => void;
  onStartFeature: (workflowId: string) => void;
}

export const WorkflowList: React.FC<WorkflowListProps> = ({ onEdit, onNew, onStartFeature }) => {
  const { reportError } = useErrorBus();
  const [workflows, setWorkflows] = useState<WorkflowWithSteps[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    loadWorkflows();
  }, []);

  const loadWorkflows = async () => {
    setLoading(true);
    try {
      const list = await invoke<WorkflowWithSteps[]>('workflow_list');
      setWorkflows(list);
      if (list.length > 0 && !selectedId) {
        setSelectedId(list[0].id);
      }
      setError(null);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setLoading(false);
    }
  };

  const handleDelete = async (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    if (!confirm('Are you sure you want to delete this custom workflow?')) return;
    try {
      await invoke('workflow_delete', { workflowId: id });
      loadWorkflows();
    } catch (err) {
      reportError(err);
    }
  };

  const handleRevert = async (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    if (!confirm('Revert this starter pack workflow to its default settings?')) return;
    try {
      await invoke('workflow_revert_to_default', { workflowId: id });
      loadWorkflows();
    } catch (err) {
      reportError(err);
    }
  };

  const handleExport = async (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      const json = await invoke<string>('workflow_export', { workflowId: id });
      const blob = new Blob([json], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `workflow-${id}.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      reportError(err);
    }
  };

  const selectedWorkflow = workflows.find((w) => w.id === selectedId);

  return (
    <div className="flex h-full w-full bg-[#08090c] text-slate-100 font-sans">
      {/* Left Column: List */}
      <div className="w-1/3 border-r border-white/5 bg-[#0d0f14]/50 flex flex-col h-full">
        <div className="p-6 flex items-center justify-between border-b border-white/5">
          <h2 className="text-xl font-bold font-display text-white tracking-wide">Workflow Library</h2>
          <button
            onClick={onNew}
            className="flex items-center gap-2 px-3 py-1.5 bg-violet-600/80 hover:bg-violet-600 hover:shadow-[0_0_15px_rgba(139,92,246,0.5)] rounded-md text-sm font-semibold transition-all duration-300 border border-violet-500/30"
          >
            <Plus className="w-4 h-4" /> New
          </button>
        </div>

        {loading ? (
          <div className="flex-1 flex items-center justify-center">
            <RefreshCw className="w-8 h-8 text-violet-500 animate-spin" />
          </div>
        ) : error ? (
          <div className="flex-1 p-6 text-rose-400 text-sm flex items-center gap-2">
            <ShieldAlert className="w-5 h-5 flex-shrink-0" />
            <span>Error: {error}</span>
          </div>
        ) : workflows.length === 0 ? (
          <div className="flex-1 p-6 text-slate-400 text-center text-sm">
            No workflows configured. Create one to begin.
          </div>
        ) : (
          <div className="flex-1 overflow-y-auto p-4 space-y-3">
            {/* Group Starter Pack */}
            <div className="space-y-2">
              <div className="text-xs uppercase tracking-wider text-slate-500 font-bold px-2">Starter Pipelines</div>
              {workflows
                .filter((w) => w.is_starter)
                .map((w) => (
                  <div
                    key={w.id}
                    onClick={() => setSelectedId(w.id)}
                    className={`p-4 rounded-lg cursor-pointer transition-all duration-300 border backdrop-blur-md ${
                      selectedId === w.id
                        ? 'bg-violet-950/20 border-violet-500/50 shadow-[0_0_15px_rgba(139,92,246,0.15)]'
                        : 'bg-white/2 hover:bg-white/5 border-white/5'
                    }`}
                  >
                    <div className="flex items-center justify-between">
                      <span className="font-semibold text-white">{w.name}</span>
                      <div className="flex items-center gap-1.5">
                        {w.schedule && (
                          <span className="text-[9px] px-1.5 py-0.5 rounded bg-violet-500/20 border border-violet-500/30 text-violet-300 flex items-center gap-1">
                            <Clock className="w-2.5 h-2.5" /> Scheduled
                          </span>
                        )}
                        <span className="text-[10px] px-2 py-0.5 rounded-full bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-bold uppercase tracking-wider">
                          Starter
                        </span>
                      </div>
                    </div>
                    <p className="text-xs text-slate-400 mt-1 line-clamp-2">{w.description}</p>
                  </div>
                ))}
            </div>

            {/* Group Custom */}
            {workflows.some((w) => !w.is_starter) && (
              <div className="space-y-2 pt-4">
                <div className="text-xs uppercase tracking-wider text-slate-500 font-bold px-2">Custom Pipelines</div>
                {workflows
                  .filter((w) => !w.is_starter)
                  .map((w) => (
                    <div
                      key={w.id}
                      onClick={() => setSelectedId(w.id)}
                      className={`p-4 rounded-lg cursor-pointer transition-all duration-300 border backdrop-blur-md ${
                        selectedId === w.id
                          ? 'bg-violet-950/20 border-violet-500/50 shadow-[0_0_15px_rgba(139,92,246,0.15)]'
                          : 'bg-white/2 hover:bg-white/5 border-white/5'
                      }`}
                    >
                      <div className="flex items-center justify-between">
                        <span className="font-semibold text-white">{w.name}</span>
                        <div className="flex items-center gap-1.5">
                          {w.schedule && (
                            <span className="text-[9px] px-1.5 py-0.5 rounded bg-violet-500/20 border border-violet-500/30 text-violet-300 flex items-center gap-1">
                              <Clock className="w-2.5 h-2.5" /> Scheduled
                            </span>
                          )}
                          <span className="text-[10px] px-2 py-0.5 rounded-full bg-cyan-500/10 text-cyan-400 border border-cyan-500/20 font-bold uppercase tracking-wider">
                            Custom
                          </span>
                        </div>
                      </div>
                      <p className="text-xs text-slate-400 mt-1 line-clamp-2">{w.description}</p>
                    </div>
                  ))}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Right Column: Preview Detail */}
      <div className="flex-1 flex flex-col h-full bg-[#08090c] p-8 overflow-y-auto">
        {selectedWorkflow ? (
          <div className="space-y-8 max-w-3xl">
            {/* Header Card */}
            <div className="p-6 rounded-xl border border-white/5 bg-white/[0.02] backdrop-blur-xl flex justify-between items-start">
              <div className="space-y-2">
                <div className="flex items-center gap-3">
                  <h1 className="text-2xl font-bold font-display text-white">{selectedWorkflow.name}</h1>
                  <span className="text-xs px-2 py-0.5 rounded bg-white/10 text-slate-300 font-mono">
                    v{selectedWorkflow.version}
                  </span>
                </div>
                <p className="text-sm text-slate-400 leading-relaxed">{selectedWorkflow.description}</p>
              </div>

              <div className="flex gap-2">
                <button
                  onClick={() => onStartFeature(selectedWorkflow.id)}
                  className="p-2.5 bg-emerald-600/80 hover:bg-emerald-600 hover:shadow-[0_0_15px_rgba(16,185,129,0.5)] rounded-lg text-white transition-all duration-300 border border-emerald-500/30"
                  title="Run Workflow"
                >
                  <Play className="w-4 h-4 fill-white" />
                </button>
                <button
                  onClick={() => onEdit(selectedWorkflow.id)}
                  className="p-2.5 bg-white/5 hover:bg-white/10 rounded-lg text-slate-300 transition-all border border-white/5"
                  title="Edit Workflow"
                >
                  <Pencil className="w-4 h-4" />
                </button>
                <button
                  onClick={(e) => handleExport(selectedWorkflow.id, e)}
                  className="p-2.5 bg-white/5 hover:bg-white/10 rounded-lg text-slate-300 transition-all border border-white/5"
                  title="Export"
                >
                  <Download className="w-4 h-4" />
                </button>
                {selectedWorkflow.is_starter ? (
                  <button
                    onClick={(e) => handleRevert(selectedWorkflow.id, e)}
                    className="p-2.5 bg-yellow-600/20 hover:bg-yellow-600/40 text-yellow-400 rounded-lg transition-all border border-yellow-500/20"
                    title="Revert to Default"
                  >
                    <RefreshCw className="w-4 h-4" />
                  </button>
                ) : (
                  <button
                    onClick={(e) => handleDelete(selectedWorkflow.id, e)}
                    className="p-2.5 bg-rose-600/20 hover:bg-rose-600 text-rose-400 rounded-lg transition-all border border-rose-500/20"
                    title="Delete"
                  >
                    <Trash className="w-4 h-4" />
                  </button>
                )}
              </div>
            </div>

            {selectedWorkflow.schedule && (
              <div className="p-4 rounded-xl border border-white/5 bg-violet-500/[0.02] flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <Clock className="w-5 h-5 text-violet-400" />
                  <div>
                    <h4 className="text-sm font-semibold text-white">Cron Execution Schedule</h4>
                    <p className="text-xs text-slate-400">
                      Cron: <code className="text-violet-300 font-mono">{selectedWorkflow.schedule.cron}</code> &bull; 
                      Title: <code className="text-cyan-300">{selectedWorkflow.schedule.title_template}</code>
                    </p>
                  </div>
                </div>
                {selectedWorkflow.schedule.next_run_at && (
                  <div className="text-right">
                    <span className="text-[10px] text-slate-500 uppercase tracking-wider font-bold block mb-0.5">Next Run</span>
                    <div className="text-xs text-slate-300 font-mono bg-black/30 border border-white/5 px-2 py-1 rounded">
                      {new Date(selectedWorkflow.schedule.next_run_at * 1000).toLocaleString()}
                    </div>
                  </div>
                )}
              </div>
            )}

            {/* Steps Timeline preview */}
            <div className="space-y-4">
              <h3 className="text-lg font-semibold font-display text-white tracking-wide">Steps Configuration</h3>
              <div className="relative border-l border-white/5 ml-4 pl-8 space-y-6">
                {selectedWorkflow.steps.map((step, idx) => {
                  let badgeColor = 'bg-cyan-500/10 text-cyan-400 border-cyan-500/20';
                  let icon = <Cpu className="w-4 h-4" />;
                  if (step.kind === 'parallel') {
                    badgeColor = 'bg-violet-500/10 text-violet-400 border-violet-500/20';
                    icon = <GitBranch className="w-4 h-4" />;
                  } else if (step.kind === 'gate') {
                    badgeColor = 'bg-amber-500/10 text-amber-400 border-amber-500/20 shadow-[0_0_10px_rgba(245,158,11,0.15)]';
                    icon = <ShieldAlert className="w-4 h-4" />;
                  }

                  return (
                    <div key={step.id} className="relative group">
                      {/* Circle node connector */}
                      <span className="absolute -left-[41px] top-1.5 flex items-center justify-center w-6 h-6 rounded-full bg-[#0d0f14] border border-white/10 group-hover:border-violet-500 transition-colors duration-300">
                        <span className="text-[10px] text-slate-400 group-hover:text-violet-400 font-bold">{idx + 1}</span>
                      </span>

                      <div className="p-5 rounded-lg border border-white/5 bg-white/[0.01] hover:bg-white/[0.03] transition-all duration-300">
                        <div className="flex items-center gap-3">
                          <span className="font-semibold text-white tracking-wide text-sm">{step.title}</span>
                          <span className={`text-[10px] flex items-center gap-1 px-2.5 py-0.5 rounded-full border font-bold uppercase tracking-wider ${badgeColor}`}>
                            {icon} {step.kind}
                          </span>
                          {step.agent_kind && (
                            <span className="text-[10px] px-2 py-0.5 rounded bg-white/5 text-slate-400 font-mono">
                              {step.agent_kind}
                            </span>
                          )}
                        </div>

                        {step.prompt_template && (
                          <div className="mt-3 text-xs text-slate-400 font-mono bg-[#050608] p-3 rounded border border-white/[0.02] max-h-24 overflow-y-auto whitespace-pre-wrap leading-relaxed">
                            {step.prompt_template}
                          </div>
                        )}

                        <div className="mt-3 flex items-center gap-4 text-[10px] text-slate-500 font-semibold uppercase tracking-wider">
                          <span>Artifact: {step.artifact_mode}</span>
                          {step.on_failure && <span>On Failure: {step.on_failure}</span>}
                          {step.max_iterations && <span>Max Loops: {step.max_iterations}</span>}
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          </div>
        ) : (
          <div className="h-full flex flex-col items-center justify-center text-slate-500 text-sm">
            <Cpu className="w-12 h-12 text-slate-700 mb-3 animate-pulse" />
            Select a workflow to preview details.
          </div>
        )}
      </div>
    </div>
  );
};
