import { Modal, SectionCard, StatusBadge } from 'demeteo';
import { AlertTriangle } from 'lucide-react';

/**
 * Modal is `fixed inset-0` and portals to `document.body`, so it fills
 * whatever surface it renders on and stacks over anything already there.
 * Its card is therefore configured `cardMode: "single"` — two open modals
 * on one page would sit on top of each other.
 */

/** A Gate awaiting a human decision — the dialog the product opens most. */
export const GateDialog = () => (
  <Modal onClose={() => {}} className="w-full max-w-lg mx-4">
    <div className="glass-panel p-6">
      <div className="flex items-center gap-3 mb-4">
        <AlertTriangle className="w-5 h-5 text-amber-400 shrink-0" />
        <h2 className="font-heading text-lg font-semibold text-white">Review gate</h2>
        <StatusBadge status="gated" variant="pill" className="ml-auto" />
      </div>
      <p className="text-sm text-slate-400 leading-relaxed">
        The worktree for <span className="font-mono text-slate-200">feat/ssh-pool</span> is
        ready to merge back, but two files conflict with the feature branch.
        Approve to merge, or send the Step back for rework.
      </p>
      <div className="mt-4 rounded-lg bg-black/40 border border-white/5 p-3">
        <div className="text-[10px] font-mono text-slate-500 uppercase tracking-widest mb-1.5">
          Conflicting files
        </div>
        <ul className="text-xs font-mono text-slate-300 space-y-1">
          <li>crates/demeteo-core/src/ports/execution.rs</li>
          <li>src/components/FeatureDetail/RunView.tsx</li>
        </ul>
      </div>
      <div className="mt-5 flex justify-end gap-2">
        <button className="px-4 py-2 rounded-lg text-sm text-slate-300 bg-white/5 border border-white/10 hover:bg-white/10 transition-all">
          Send back
        </button>
        <button className="px-4 py-2 rounded-lg text-sm font-medium text-white bg-violet-500 border border-violet-400/50 hover:bg-violet-500 transition-all">
          Approve merge
        </button>
      </div>
    </div>
  </Modal>
);

/** A compact confirmation — the smallest shape the backdrop is used for. */
export const Confirm = () => (
  <Modal onClose={() => {}} className="w-full max-w-sm mx-4">
    <SectionCard title="Stop this run?">
      <p className="text-sm text-slate-400">
        Three Subtasks are still running. Their worktrees are kept.
      </p>
      <div className="mt-4 flex justify-end gap-2">
        <button className="px-3 py-1.5 rounded-lg text-xs text-slate-300 bg-white/5 border border-white/10">
          Cancel
        </button>
        <button className="px-3 py-1.5 rounded-lg text-xs font-medium text-ruby-200 bg-ruby-500/20 border border-ruby-500/40">
          Stop run
        </button>
      </div>
    </SectionCard>
  </Modal>
);
