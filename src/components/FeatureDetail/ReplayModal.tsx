import { RotateCcw } from 'lucide-react';
import { Modal } from '../ui/Modal';
import type { ReplayTarget } from './useRerunActions';

/**
 * Confirm a rewind to `target`.
 *
 * It confirms the rewind and nothing else: harness, model and effort params
 * write the feature-wide tier, which every step after the target resolves
 * against. A node is re-pointed on the inspector's Assignment control, whose
 * blast radius is the node it is on.
 */
export function ReplayModal({
  target,
  status,
  onClose,
  onConfirm,
}: {
  target: ReplayTarget | null;
  status: string;
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
          <h3 className="text-sm font-bold text-white font-heading tracking-wide">
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
