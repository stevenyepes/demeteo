import { RotateCcw } from 'lucide-react';
import { Modal } from '../ui/Modal';
import { EFFORT_LABELS, type EffortLevel } from '../../lib/effortLevels';
import type { HarnessOverrides } from './useHarnessOverrides';
import type { ReplayTarget } from './useRerunActions';

/** Confirm a rewind to `target`, re-pinning the harness/model/effort it runs with. */
export function ReplayModal({
  target,
  status,
  overrides,
  onClose,
  onConfirm,
}: {
  target: ReplayTarget | null;
  status: string;
  overrides: HarnessOverrides;
  onClose: () => void;
  onConfirm: () => void;
}) {
  if (!target) return null;
  return (
    <Modal onClose={onClose} backdropClassName="bg-black/60" className="bg-[#0d0f14] border border-white/10 rounded-2xl p-6 max-w-md w-full mx-4 shadow-[0_0_40px_rgba(0,0,0,0.5)]">
      <div className="flex items-center gap-3 mb-4">
        <div className="w-8 h-8 rounded-full bg-cyan-500/10 border border-cyan-500/20 flex items-center justify-center">
          <RotateCcw className="w-4 h-4 text-cyan-400" />
        </div>
        <div>
          <h3 className="text-sm font-bold text-white font-display tracking-wide">
            Replay from "{target.name}"
          </h3>
          <p className="text-[10px] text-slate-500 font-mono mt-0.5">
            {target.downstreamCount > 0
              ? `${target.downstreamCount} downstream step${target.downstreamCount > 1 ? 's' : ''} will be re-executed`
              : 'Only this step will be re-executed'}
          </p>
        </div>
      </div>

      <p className="text-xs text-slate-400 mb-5 leading-relaxed">
        Current artifacts for the affected steps will be replaced.
        {status === 'running' && ' The current execution will be cancelled.'}
      </p>

      {overrides.availableAgents.length > 0 && (
        <div className="flex items-center gap-3 bg-black/20 p-2.5 rounded border border-white/5 mb-2.5">
          <label className="text-[10px] uppercase font-bold text-slate-400 shrink-0 font-mono">Harness:</label>
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
        <div className="text-[10px] text-slate-500 font-mono animate-pulse mb-5 px-1">Probing available models…</div>
      ) : overrides.availableModels.length > 0 && (
        <div className="flex items-center gap-3 bg-black/20 p-2.5 rounded border border-white/5 mb-5">
          <label className="text-[10px] uppercase font-bold text-slate-400 shrink-0 font-mono">Model:</label>
          <select
            value={overrides.selectedModel}
            onChange={(e) => overrides.setSelectedModel(e.target.value)}
            className="flex-1 min-w-0 bg-[#0d0f14] border border-white/10 rounded px-2.5 py-1.5 text-xs text-slate-200 outline-none focus:border-violet-500/50 font-mono cursor-pointer"
          >
            <option value="">Default (From Workflow)</option>
            {overrides.availableModels.map((m) => (
              <option key={m.value} value={m.value}>{m.name}</option>
            ))}
          </select>
        </div>
      )}

      <div className="flex items-center gap-3 bg-black/20 p-2.5 rounded border border-white/5 mb-5">
        <label htmlFor="replay-effort" className="text-[10px] uppercase font-bold text-slate-400 shrink-0 font-mono">Effort:</label>
        <select
          id="replay-effort"
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

      <div className="flex justify-end gap-2">
        <button
          onClick={onClose}
          className="px-4 py-2 bg-white/5 hover:bg-white/10 rounded-lg text-xs font-semibold transition"
        >
          Cancel
        </button>
        <button
          onClick={onConfirm}
          className="flex items-center gap-1.5 px-4 py-2 bg-emerald-600 hover:bg-emerald-500 hover:shadow-[0_0_20px_rgba(16,185,129,0.5)] rounded-lg text-xs font-bold text-white transition duration-300 shadow-[0_0_15px_rgba(16,185,129,0.3)]"
        >
          <RotateCcw className="w-3 h-3" /> Replay
        </button>
      </div>
    </Modal>
  );
}
