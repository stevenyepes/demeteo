import { AlertTriangle, RefreshCw } from 'lucide-react';
import type { StepExecution } from '../../types';
import { EFFORT_LABELS, type EffortLevel } from '../../lib/effortLevels';
import type { HarnessOverrides } from './useHarnessOverrides';

interface StepRerunPanelProps {
  step: StepExecution;
  /** True when the step died because the machine could not run the command
   *  at all, rather than because the change is wrong. */
  isEnvironmentFailure: boolean;
  activePredecessor: StepExecution | null;
  isBlockedByPredecessor: boolean;
  overrides: HarnessOverrides;
  onRetry: () => void;
}

/** What a failed step offers: change the harness/model/effort, then retry. */
export function StepRerunPanel({
  step,
  isEnvironmentFailure,
  activePredecessor,
  isBlockedByPredecessor,
  overrides,
  onRetry,
}: StepRerunPanelProps) {
  return (
    <div className="mt-4 p-4 rounded bg-rose-500/5 border border-rose-500/20 flex flex-col gap-3">
      <div className="flex justify-between items-center">
        <div className="text-xs text-rose-400 font-semibold uppercase tracking-wide">
          {isEnvironmentFailure
            ? 'Fix the machine first — retrying before that fails the same way.'
            : 'Step failed. You can change harness/model and retry.'}
        </div>
        <button
          onClick={onRetry}
          disabled={isBlockedByPredecessor}
          title={
            isBlockedByPredecessor
              ? `Step '${activePredecessor?.step_id}' is still ${activePredecessor?.status}. Wait for it to finish before retrying.`
              : 'Re-run this step from scratch'
          }
          className="flex items-center gap-1.5 px-3 py-1.5 bg-rose-600 hover:bg-rose-500 disabled:bg-rose-900/40 disabled:hover:bg-rose-900/40 disabled:cursor-not-allowed text-white rounded text-xs font-bold transition shadow-[0_0_10px_rgba(239,68,68,0.4)] disabled:shadow-none"
        >
          <RefreshCw className="w-3 h-3 animate-pulse" /> Retry Step
        </button>
      </div>

      {isBlockedByPredecessor && activePredecessor && (
        <div
          data-testid="retry-blocked-banner"
          className="flex items-start gap-2 px-3 py-2 rounded bg-amber-500/5 border border-amber-500/20 text-[11px] text-amber-400 font-mono"
          title={`Cannot retry while '${activePredecessor.step_id}' is ${activePredecessor.status}`}
        >
          <AlertTriangle className="w-3 h-3 mt-0.5 shrink-0" />
          <span>
            Blocked: <span className="font-semibold">{activePredecessor.step_id}</span> is still {activePredecessor.status}. Wait for it to finish before retrying.
          </span>
        </div>
      )}

      {overrides.availableAgents.length > 0 && (
        <div className="flex items-center gap-3 bg-black/20 p-2.5 rounded border border-white/5">
          <label className="text-[10px] uppercase font-bold text-slate-400 shrink-0 font-mono">Run with Harness:</label>
          <select
            value={overrides.selectedAgent}
            onChange={(e) => overrides.onAgentChange(e.target.value)}
            className="flex-1 min-w-0 bg-[#0d0f14] border border-white/10 rounded px-2.5 py-1.5 text-xs text-slate-200 outline-none focus:border-violet-500/50 font-mono cursor-pointer capitalize"
          >
            <option value="">Default ({overrides.featureAgentKind.replace(/-/g, ' ')})</option>
            {overrides.availableAgents.map((a) => (
              <option key={a} value={a}>{a.replace(/-/g, ' ')}</option>
            ))}
          </select>
        </div>
      )}

      {overrides.isLoadingModels ? (
        <div className="text-[10px] text-slate-500 font-mono animate-pulse">
          Probing available models...
        </div>
      ) : overrides.availableModels.length > 0 ? (
        <div className="flex items-center gap-3 bg-black/20 p-2.5 rounded border border-white/5">
          <label className="text-[10px] uppercase font-bold text-slate-400 shrink-0 font-mono">Run with Model:</label>
          <select
            value={overrides.selectedModel}
            onChange={(e) => overrides.setSelectedModel(e.target.value)}
            className="flex-1 min-w-0 bg-[#0d0f14] border border-white/10 rounded px-2.5 py-1.5 text-xs text-slate-200 outline-none focus:border-violet-500/50 font-mono cursor-pointer"
          >
            <option value="">Default (From Workflow)</option>
            {overrides.availableModels.map((m) => (
              <option key={m.value} value={m.value}>
                {m.name}
              </option>
            ))}
          </select>
        </div>
      ) : null}

      <div className="flex items-center gap-3 bg-black/20 p-2.5 rounded border border-white/5">
        <label htmlFor={`retry-effort-${step.id}`} className="text-[10px] uppercase font-bold text-slate-400 shrink-0 font-mono">Run with Effort:</label>
        <select
          id={`retry-effort-${step.id}`}
          value={overrides.selectedEffort}
          onChange={(e) => overrides.setSelectedEffort(e.target.value as EffortLevel | '')}
          disabled={overrides.retryEffortLevels.length === 0}
          title={overrides.retryEffortLevels.length === 0 ? `${(overrides.selectedAgent || overrides.featureAgentKind).replace(/-/g, ' ')} does not support effort selection` : undefined}
          className="flex-1 min-w-0 bg-[#0d0f14] border border-white/10 rounded px-2.5 py-1.5 text-xs text-slate-200 outline-none focus:border-violet-500/50 font-mono cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
        >
          <option value="">{overrides.retryEffortLevels.length === 0 ? 'Not supported' : 'Keep current effort'}</option>
          {overrides.retryEffortLevels.map((l) => (
            <option key={l} value={l}>{EFFORT_LABELS[l]}</option>
          ))}
        </select>
      </div>
    </div>
  );
}
