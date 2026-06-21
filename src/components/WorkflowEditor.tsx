import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { WorkflowWithSteps, StepConfig } from '../types';
import { ArrowLeft, Plus, Trash, ChevronUp, ChevronDown, Save } from 'lucide-react';

interface WorkflowEditorProps {
  workflowId: string | null; // null means create new
  onBack: () => void;
  onSaved: () => void;
}

const AGENT_KINDS = ['opencode', 'hermes', 'claude-code', 'antigravity'];

export const WorkflowEditor: React.FC<WorkflowEditorProps> = ({ workflowId, onBack, onSaved }) => {
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [steps, setSteps] = useState<StepConfig[]>([]);
  const [note, setNote] = useState('');
  const [version, setVersion] = useState(1);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (workflowId) {
      loadWorkflow(workflowId);
    } else {
      // Seed a default step
      setSteps([
        {
          id: 'step-1',
          kind: 'agent',
          title: 'Research and Plan',
          agent_kind: null,
          prompt_template: 'Research {{feature_description}}',
          artifact_mode: 'full',
          on_failure: null,
          max_iterations: null,
        },
      ]);
    }
  }, [workflowId]);

  const loadWorkflow = async (id: string) => {
    setLoading(true);
    try {
      const w = await invoke<WorkflowWithSteps>('workflow_get', { workflowId: id });
      setName(w.name);
      setDescription(w.description);
      setSteps(w.steps);
      setVersion(w.version);
    } catch (err: any) {
      console.error(err);
    } finally {
      setLoading(false);
    }
  };

  const handleAddStep = (type: 'agent' | 'parallel' | 'gate') => {
    const newId = `step-${Date.now()}`;
    const newStep: StepConfig = {
      id: newId,
      kind: type,
      title: type === 'agent' ? 'Run Coding Agent' : type === 'parallel' ? 'Decompose & Implement' : 'User Approval Gate',
      agent_kind: type === 'gate' ? null : 'opencode',
      prompt_template: type === 'gate' ? null : 'Implement task based on requirements',
      artifact_mode: type === 'gate' ? 'none' : 'full',
      on_failure: null,
      max_iterations: null,
    };
    setSteps([...steps, newStep]);
  };

  const handleRemoveStep = (index: number) => {
    setSteps(steps.filter((_, i) => i !== index));
  };

  const handleUpdateStep = (index: number, fields: Partial<StepConfig>) => {
    setSteps(
      steps.map((s, i) => (i === index ? { ...s, ...fields } as StepConfig : s))
    );
  };

  const moveStep = (index: number, direction: 'up' | 'down') => {
    const nextIndex = direction === 'up' ? index - 1 : index + 1;
    if (nextIndex < 0 || nextIndex >= steps.length) return;
    const newSteps = [...steps];
    const temp = newSteps[index];
    newSteps[index] = newSteps[nextIndex];
    newSteps[nextIndex] = temp;
    setSteps(newSteps);
  };

  const handleSave = async () => {
    if (!name.trim()) {
      alert('Please specify a workflow name.');
      return;
    }
    if (steps.length === 0) {
      alert('Workflow must contain at least one step.');
      return;
    }
    const lastStep = steps[steps.length - 1];
    if (lastStep.kind === 'gate') {
      alert('A user gate step cannot be the last step in a workflow pipeline.');
      return;
    }

    setLoading(true);
    try {
      if (workflowId) {
        // Update
        await invoke('workflow_update', {
          workflowId,
          name,
          description,
          steps,
          note: note || `Updated to version ${version + 1}`,
        });
      } else {
        // Create
        await invoke('workflow_create', {
          name,
          description,
          steps,
        });
      }
      onSaved();
    } catch (err: any) {
      alert(err.toString());
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="h-full w-full bg-[#08090c] text-slate-100 flex flex-col font-sans">
      {/* Top Action Bar */}
      <div className="p-6 border-b border-white/5 bg-[#0d0f14]/80 flex items-center justify-between backdrop-blur-md">
        <div className="flex items-center gap-4">
          <button
            onClick={onBack}
            className="p-2 bg-white/5 hover:bg-white/10 rounded-lg text-slate-400 hover:text-white transition"
          >
            <ArrowLeft className="w-4 h-4" />
          </button>
          <div>
            <h1 className="text-xl font-bold font-display text-white tracking-wide">
              {workflowId ? 'Modify Workflow' : 'Create Workflow'}
            </h1>
            <p className="text-xs text-slate-400">
              {workflowId ? `Editing version ${version}` : 'Configure feature pipelines and dispatch parameters'}
            </p>
          </div>
        </div>

        <div className="flex items-center gap-3">
          <button
            onClick={handleSave}
            disabled={loading}
            className="flex items-center gap-2 px-4 py-2 bg-emerald-600/80 hover:bg-emerald-600 hover:shadow-[0_0_15px_rgba(16,185,129,0.5)] rounded-lg text-sm font-semibold transition-all duration-300 border border-emerald-500/30"
          >
            <Save className="w-4 h-4" /> {workflowId ? 'Save Version' : 'Create'}
          </button>
        </div>
      </div>

      {loading && steps.length === 0 ? (
        <div className="flex-1 flex items-center justify-center">
          <ChevronUp className="w-8 h-8 text-violet-500 animate-spin" />
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto p-8 max-w-4xl mx-auto w-full space-y-6">
          {/* Metadata Section */}
          <div className="p-6 rounded-xl border border-white/5 bg-white/[0.02] backdrop-blur-xl space-y-4">
            <h3 className="text-sm uppercase tracking-wider text-slate-400 font-bold">Metadata</h3>
            <div className="grid grid-cols-1 gap-4">
              <div>
                <label className="block text-xs text-slate-400 mb-1 font-semibold uppercase">Pipeline Name</label>
                <input
                  type="text"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="e.g. Standard Feature Pipeline"
                  className="w-full bg-[#0d0f14] border border-white/10 focus:border-violet-500 rounded-lg p-3 text-sm text-white focus:outline-none transition font-sans"
                />
              </div>
              <div>
                <label className="block text-xs text-slate-400 mb-1 font-semibold uppercase">Description</label>
                <textarea
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder="Describe the workflow purpose, target project size, and best use cases."
                  rows={2}
                  className="w-full bg-[#0d0f14] border border-white/10 focus:border-violet-500 rounded-lg p-3 text-sm text-white focus:outline-none transition resize-none font-sans"
                />
              </div>
              {workflowId && (
                <div>
                  <label className="block text-xs text-slate-400 mb-1 font-semibold uppercase">Revision Note (Optional)</label>
                  <input
                    type="text"
                    value={note}
                    onChange={(e) => setNote(e.target.value)}
                    placeholder="e.g. Optimized implementation prompt template"
                    className="w-full bg-[#0d0f14] border border-white/10 focus:border-violet-500 rounded-lg p-3 text-sm text-white focus:outline-none transition font-sans"
                  />
                </div>
              )}
            </div>
          </div>

          {/* Steps Section */}
          <div className="space-y-4">
            <div className="flex justify-between items-center">
              <h3 className="text-sm uppercase tracking-wider text-slate-400 font-bold">Pipeline Steps</h3>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => handleAddStep('agent')}
                  className="flex items-center gap-1.5 px-3 py-1 bg-cyan-500/10 text-cyan-400 hover:bg-cyan-500/20 border border-cyan-500/20 rounded text-xs font-bold transition"
                >
                  <Plus className="w-3.5 h-3.5" /> Agent
                </button>
                <button
                  type="button"
                  onClick={() => handleAddStep('parallel')}
                  className="flex items-center gap-1.5 px-3 py-1 bg-violet-500/10 text-violet-400 hover:bg-violet-500/20 border border-violet-500/20 rounded text-xs font-bold transition"
                >
                  <Plus className="w-3.5 h-3.5" /> Parallel
                </button>
                <button
                  type="button"
                  onClick={() => handleAddStep('gate')}
                  className="flex items-center gap-1.5 px-3 py-1 bg-amber-500/10 text-amber-400 hover:bg-amber-500/20 border border-amber-500/20 rounded text-xs font-bold transition"
                >
                  <Plus className="w-3.5 h-3.5" /> Gate
                </button>
              </div>
            </div>

            {steps.length === 0 ? (
              <div className="p-8 border border-dashed border-white/10 rounded-xl text-center text-sm text-slate-500">
                Click a button above to insert steps into this workflow.
              </div>
            ) : (
              <div className="space-y-4">
                {steps.map((step, idx) => {
                  let leftBorder = 'border-l-cyan-500/80';
                  if (step.kind === 'parallel') leftBorder = 'border-l-violet-500/80';
                  if (step.kind === 'gate') leftBorder = 'border-l-amber-500/80';

                  return (
                    <div
                      key={step.id}
                      className={`rounded-xl border border-white/5 bg-white/[0.01] overflow-hidden border-l-4 ${leftBorder}`}
                    >
                      {/* Step Header */}
                      <div className="p-4 border-b border-white/5 bg-white/[0.01] flex items-center justify-between">
                        <div className="flex items-center gap-3">
                          <span className="text-xs font-bold px-2 py-0.5 rounded bg-white/5 text-slate-400">
                            {idx + 1}
                          </span>
                          <input
                            type="text"
                            value={step.title}
                            onChange={(e) => handleUpdateStep(idx, { title: e.target.value })}
                            className="bg-transparent border-b border-transparent hover:border-white/20 focus:border-violet-500 focus:outline-none text-sm font-semibold text-white px-1 py-0.5 w-64 transition"
                          />
                        </div>

                        <div className="flex items-center gap-1">
                          <button
                            type="button"
                            onClick={() => moveStep(idx, 'up')}
                            disabled={idx === 0}
                            className="p-1.5 hover:bg-white/5 disabled:opacity-30 rounded text-slate-400 hover:text-white transition"
                          >
                            <ChevronUp className="w-4 h-4" />
                          </button>
                          <button
                            type="button"
                            onClick={() => moveStep(idx, 'down')}
                            disabled={idx === steps.length - 1}
                            className="p-1.5 hover:bg-white/5 disabled:opacity-30 rounded text-slate-400 hover:text-white transition"
                          >
                            <ChevronDown className="w-4 h-4" />
                          </button>
                          <button
                            type="button"
                            onClick={() => handleRemoveStep(idx)}
                            className="p-1.5 hover:bg-rose-500/10 text-slate-400 hover:text-rose-400 rounded transition"
                          >
                            <Trash className="w-4 h-4" />
                          </button>
                        </div>
                      </div>

                      {/* Step Body */}
                      <div className="p-5 space-y-4">
                        <div className="grid grid-cols-2 gap-4">
                          {/* Step Kind Selector */}
                          <div>
                            <label className="block text-[10px] text-slate-400 mb-1 uppercase font-semibold">Step Type</label>
                            <select
                              value={step.kind}
                              onChange={(e) => handleUpdateStep(idx, { kind: e.target.value })}
                              className="w-full bg-[#0d0f14] border border-white/10 rounded-md p-2 text-xs text-white focus:outline-none focus:border-violet-500"
                            >
                              <option value="agent">Agent (Sequential Dispatch)</option>
                              <option value="parallel">Parallel (Worktree Split & Merge)</option>
                              <option value="gate">Gate (Manual Decisional Stop)</option>
                            </select>
                          </div>

                          {/* Agent kind Override */}
                          {step.kind !== 'gate' && (
                            <div>
                              <label className="block text-[10px] text-slate-400 mb-1 uppercase font-semibold">
                                Dispatch Override
                              </label>
                              <select
                                value={step.agent_kind || ''}
                                onChange={(e) => handleUpdateStep(idx, { agent_kind: e.target.value || null })}
                                className="w-full bg-[#0d0f14] border border-white/10 rounded-md p-2 text-xs text-white focus:outline-none focus:border-violet-500"
                              >
                                <option value="">Project Default</option>
                                {AGENT_KINDS.map((ak) => (
                                  <option key={ak} value={ak}>
                                    {ak}
                                  </option>
                                ))}
                              </select>
                            </div>
                          )}
                        </div>

                        {/* Prompt Template */}
                        {step.kind !== 'gate' && (
                          <div>
                            <label className="block text-[10px] text-slate-400 mb-1 uppercase font-semibold">
                              Prompt Instruction Template
                            </label>
                            <textarea
                              value={step.prompt_template || ''}
                              onChange={(e) => handleUpdateStep(idx, { prompt_template: e.target.value })}
                              placeholder="Describe the instructions for this step. Use {{feature_description}} to represent the user description."
                              rows={3}
                              className="w-full bg-[#0d0f14] border border-white/10 rounded-md p-3 text-xs text-white focus:outline-none focus:border-violet-500 font-mono resize-none leading-relaxed"
                            />
                          </div>
                        )}

                        <div className="grid grid-cols-3 gap-4">
                          {/* Artifact Mode */}
                          {step.kind !== 'gate' && (
                            <div>
                              <label className="block text-[10px] text-slate-400 mb-1 uppercase font-semibold">
                                Artifact Mode
                              </label>
                              <select
                                value={step.artifact_mode}
                                onChange={(e) => handleUpdateStep(idx, { artifact_mode: e.target.value })}
                                className="w-full bg-[#0d0f14] border border-white/10 rounded-md p-2 text-xs text-white focus:outline-none focus:border-violet-500"
                              >
                                <option value="full">Full Artifact</option>
                                <option value="summary_only">Summary Only</option>
                                <option value="none">No Artifact</option>
                              </select>
                            </div>
                          )}

                          {/* Loop failure target */}
                          <div>
                            <label className="block text-[10px] text-slate-400 mb-1 uppercase font-semibold">
                              On Failure (Loopback)
                            </label>
                            <select
                              value={step.on_failure || ''}
                              onChange={(e) => handleUpdateStep(idx, { on_failure: e.target.value || null })}
                              className="w-full bg-[#0d0f14] border border-white/10 rounded-md p-2 text-xs text-white focus:outline-none focus:border-violet-500"
                            >
                              <option value="">Abort pipeline</option>
                              {steps
                                .filter((s) => s.id !== step.id)
                                .map((s) => (
                                  <option key={s.id} value={s.id}>
                                    Jump to: {s.title}
                                  </option>
                                ))}
                            </select>
                          </div>

                          {/* Max iterations */}
                          {step.on_failure && (
                            <div>
                              <label className="block text-[10px] text-slate-400 mb-1 uppercase font-semibold">
                                Max Loop Iterations
                              </label>
                              <input
                                type="number"
                                min={1}
                                max={10}
                                value={step.max_iterations || ''}
                                onChange={(e) => handleUpdateStep(idx, { max_iterations: parseInt(e.target.value) || null })}
                                placeholder="None"
                                className="w-full bg-[#0d0f14] border border-white/10 rounded-md p-2 text-xs text-white focus:outline-none focus:border-violet-500"
                              />
                            </div>
                          )}
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};
