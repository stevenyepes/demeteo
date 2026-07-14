import { Workflow as WorkflowIcon, ShieldAlert, AlertTriangle, RotateCw, ChevronDown, ChevronUp } from 'lucide-react';
import type { StepConfig } from '../../types';
import { EFFORT_LABELS } from '../../lib/effortLevels';
import { HarnessModelPicker } from '../ui/HarnessModelPicker';
import { useSettings, ovKey, EMPTY_ROW, isOverrideActive, WF_LEVEL } from './ProjectSettingsContext';

/**
 * One override row — the workflow-level row (`step === null`) or a per-step
 * one. The harness / model / effort markup is the shared
 * {@link HarnessModelPicker}, so all three controls stay identical to the
 * launch modal and the wizards.
 */
function OverrideRow({ wf, step }: { wf: { id: string; steps: StepConfig[] }; step: StepConfig | null }) {
  const s = useSettings();
  const stepId = step ? step.id : WF_LEVEL;
  const key = ovKey(wf.id, stepId);
  const ov = s.overrides[key] ?? EMPTY_ROW;
  const models = s.rowModels[key] ?? [];
  const modelsLoading = Boolean(s.rowModelsLoading[key]);
  const effectiveAgent = s.effectiveAgentForRow(wf.id, step);
  const inhA = step ? s.inheritedAgent(wf.id, step) : (s.defaultAgentKind || '');
  const inhM = step ? s.inheritedModel(wf.id, step) : (s.defaultModel || '');
  // The workflow-level row inherits from the project default; a step row also
  // inherits through the workflow-level row and the workflow author's setting.
  const inhE = step ? s.inheritedEffort(wf.id, step) : (s.defaultEffort || 'high');
  const agentPlaceholder = step ? `Inherit${inhA ? ` · ${inhA.replace(/-/g, ' ')}` : ' · built-in'}` : 'Project default';
  const modelPlaceholder = inhM ? `Inherit · ${inhM}` : 'Agent default model';

  return (
    <HarnessModelPicker
      agentKinds={s.overrideAgentKinds}
      models={models}
      modelsLoading={modelsLoading}
      agentKind={ov.agent_kind ?? ''}
      model={ov.model ?? ''}
      effort={ov.effort ?? ''}
      inheritedAgentKind={effectiveAgent}
      effortLevels={s.effortLevelsFor(effectiveAgent)}
      onAgentKindChange={k => s.handleAgentChange(wf.id, stepId, step, k)}
      onModelChange={m => s.handleModelChange(wf.id, stepId, m)}
      onEffortChange={e => s.handleEffortChange(wf.id, stepId, e)}
      onClear={() => s.handleClearRow(wf.id, stepId)}
      agentPlaceholder={agentPlaceholder}
      modelPlaceholder={modelPlaceholder}
      effortPlaceholder={`Inherit · ${EFFORT_LABELS[inhE]}`}
      saved={Boolean(s.savedPulse[key])}
    />
  );
}

export function OverridesTab() {
  const s = useSettings();

  return (
    <div className="space-y-4 animate-fadeIn">
      <div className="glass-panel p-6 rounded-xl space-y-2">
        <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider flex items-center gap-2"><WorkflowIcon className="w-4 h-4 text-violet-400" /> Workflow &amp; Step Harness, Model &amp; Effort</h3>
        <p className="text-xs text-slate-400 leading-relaxed">Workflows are shared across projects. Pin a coding agent (<span className="text-slate-300">harness</span>), a model and a reasoning <span className="text-slate-300">effort</span> for a whole workflow — or a single step — <span className="text-white font-medium">when it runs in {s.activeProject.name}</span>. Models are probed live from your {s.computeType === 'remote' ? 'remote machine' : 'local machine'}, so you only pick what's actually available; effort levels come from what each agent declares.</p>
        <p className="text-[11px] text-slate-500 leading-relaxed">Precedence, most specific first: a choice made at launch → a step override here → the workflow author's step setting → a workflow override here → the project default. Expand a workflow to override individual steps. Changes save instantly.</p>
      </div>

      {s.overridesError && (
        <div className="bg-ruby-500/10 border border-ruby-500/30 p-3 rounded-lg flex items-start gap-3">
          <ShieldAlert className="w-5 h-5 text-ruby-400 shrink-0" />
          <span className="text-sm text-ruby-200">{s.overridesError}</span>
        </div>
      )}

      {s.computeType === 'remote' && !s.remoteHost && (
        <div className="bg-amber-500/10 border border-amber-500/20 p-3 rounded-lg flex items-start gap-3">
          <AlertTriangle className="w-5 h-5 text-amber-400 shrink-0" />
          <span className="text-sm text-amber-200">Select a remote machine in <span className="font-medium">General &amp; Repositories</span> to probe available models.</span>
        </div>
      )}

      {s.isLoadingOverrides ? (
        <div className="flex items-center justify-center py-16"><RotateCw className="w-6 h-6 text-cyan-400 animate-spin" /></div>
      ) : s.workflows.length === 0 ? (
        <div className="text-center py-16 border border-dashed border-white/10 rounded-xl bg-black/20">
          <WorkflowIcon className="w-8 h-8 text-slate-600 mx-auto mb-3" />
          <p className="text-sm font-medium text-slate-400">No workflows found</p>
          <p className="text-xs text-slate-500 mt-1">Create a workflow first, then return here to override its harness and model.</p>
        </div>
      ) : (
        <div className="space-y-3">
          {s.workflows.map(wf => {
            const count = s.workflowOverrideCount(wf);
            const isActive = count > 0;
            const expanded = Boolean(s.expandedWf[wf.id]);
            const agentSteps = wf.steps.filter(st => st.kind !== 'gate');
            const wfLevel = s.overrides[ovKey(wf.id, WF_LEVEL)];
            return (
              <div key={wf.id} className={`glass-panel rounded-xl border transition-all ${isActive ? 'border-violet-500/30 bg-violet-500/[0.03]' : 'border-white/5'}`}>
                <div role="button" tabIndex={0} onClick={() => s.toggleWorkflowExpanded(wf)} onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); s.toggleWorkflowExpanded(wf); } }} className="flex items-center gap-3 p-5 cursor-pointer select-none">
                  <div className={`w-8 h-8 rounded-lg flex items-center justify-center shrink-0 border ${isActive ? 'bg-violet-500/10 border-violet-500/30 text-violet-300' : 'bg-white/5 border-white/10 text-slate-400'}`}>
                    <WorkflowIcon className="w-4 h-4" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="text-sm font-semibold text-white truncate">{wf.name}</span>
                      {isOverrideActive(wfLevel) && <span className="px-2 py-0.5 text-[9px] font-mono rounded-full bg-violet-500/10 border border-violet-500/20 text-violet-300 uppercase tracking-wider shrink-0">All steps</span>}
                      {(() => { const sc = count - (isOverrideActive(wfLevel) ? 1 : 0); return sc > 0 ? <span className="px-2 py-0.5 text-[9px] font-mono rounded-full bg-cyan-500/10 border border-cyan-500/20 text-cyan-300 uppercase tracking-wider shrink-0">{sc} step{sc !== 1 ? 's' : ''}</span> : null; })()}
                    </div>
                    {wf.description && <p className="text-[11px] text-slate-500 mt-0.5 line-clamp-1">{wf.description}</p>}
                  </div>
                  <span className="text-[10px] text-slate-500 font-mono shrink-0">{agentSteps.length} step{agentSteps.length !== 1 ? 's' : ''}</span>
                  {expanded ? <ChevronUp className="w-4 h-4 text-slate-500 shrink-0" /> : <ChevronDown className="w-4 h-4 text-slate-500 shrink-0" />}
                </div>
                {expanded && (
                  <div className="px-5 pb-5 space-y-5 border-t border-white/5 pt-4">
                    <div>
                      <div className="flex items-center gap-2 mb-2"><span className="text-[10px] font-bold text-violet-300/80 uppercase tracking-wider">Applies to all steps</span><div className="h-px flex-1 bg-white/5" /></div>
                      <OverrideRow wf={wf} step={null} />
                    </div>
                    {agentSteps.length > 0 && (
                      <div>
                        <div className="flex items-center gap-2 mb-3"><span className="text-[10px] font-bold text-slate-400 uppercase tracking-wider">Per-step overrides</span><div className="h-px flex-1 bg-white/5" /></div>
                        <div className="space-y-4">
                          {agentSteps.map((step, idx) => (
                            <div key={step.id} className="rounded-lg border border-white/5 bg-black/20 p-3.5">
                              <div className="flex items-center gap-2 mb-3">
                                <span className="text-[10px] font-bold px-1.5 py-0.5 rounded bg-white/5 text-slate-400 shrink-0">{idx + 1}</span>
                                <span className="text-xs font-semibold text-slate-200 truncate">{step.title}</span>
                                <span className={`px-1.5 py-0.5 text-[9px] font-mono rounded uppercase tracking-wider shrink-0 ${step.kind === 'sequence' || step.kind === 'parallel' ? 'bg-violet-500/10 text-violet-300' : 'bg-cyan-500/10 text-cyan-300'}`}>{step.kind}</span>
                              </div>
                              <OverrideRow wf={wf} step={step} />
                            </div>
                          ))}
                        </div>
                      </div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
