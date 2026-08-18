import React from 'react';
import { AlertCircle, RefreshCw, RotateCcw, ShieldCheck, XCircle } from 'lucide-react';

import { isOutOfBandStep } from '../../../lib/featureSync';
import { TONE_TEXT } from '../../../lib/runStatus';
import type { StepAttempt } from '../../../types';
import { RerunOptions } from '../../FeatureDetail/RerunOptions';
import type { HarnessOverrides } from '../../FeatureDetail/useHarnessOverrides';
import type { NodeConfigV2, NodeRunStatus } from '../types';
import { classLabel } from './format';

/** The active ancestor blocking a manual retry/gate decision, if any. */
export interface BlockingAncestor {
  step_id: string;
  status: string;
}

/** Actions: retry / replay / stop / decide-gate, with the ancestor guard.
 *  The panel holds no run logic of its own — FeatureDetail owns the handlers
 *  and passes them in, so the canvas and the timeline drive the same paths. */
export function ActionsTab({
  node,
  run,
  hasActions,
  attempts,
  blockedBy,
  overrides,
  onRetry,
  onReplay,
  onStop,
  onDecideGate,
}: {
  node: NodeConfigV2;
  run: NodeRunStatus | null;
  hasActions: boolean;
  attempts: StepAttempt[];
  blockedBy: BlockingAncestor | null;
  /** The harness/model/effort a retry would re-pin. Absent where nobody holds
   *  them — the canvas mounts this panel outside the run view. */
  overrides?: HarnessOverrides;
  onRetry?: () => void;
  onReplay?: () => void;
  onStop?: () => void;
  onDecideGate?: () => void;
}) {
  const status = run?.status ?? 'pending';
  // Retry and Replay walk the graph from this node, and an out-of-band sync is
  // in no graph — the backend refuses both
  // (`domain::run_control::out_of_band_refusal`), so offering them is a button
  // whose only outcome is an error toast.
  const graphless = isOutOfBandStep(node.id);
  const isFailed = (status === 'failed' || status === 'interrupted') && !graphless;
  const isRunning = status === 'running' || status === 'verifying';
  const isGateWaiting = node.type === 'gate' && status === 'awaiting_gate';
  const guarded = blockedBy !== null;
  const guardMsg = blockedBy
    ? `Ancestor "${blockedBy.step_id}" is still ${blockedBy.status}. Wait for it to finish.`
    : '';

  // The policy rule the engine applied to this node's most recent failure — the
  // "which rule will apply" hint (P2.4), read straight from the attempt row.
  const lastFailed = [...attempts].reverse().find((a) => a.error_class);

  const anyAction =
    (onDecideGate && isGateWaiting) ||
    (onRetry && isFailed) ||
    (onReplay && !graphless) ||
    (onStop && isRunning);

  if (!hasActions || !anyAction) {
    return (
      <div className="flex h-full items-center justify-center px-8 text-center text-xs font-bold uppercase tracking-wider text-slate-600">
        No actions available for this node yet.
      </div>
    );
  }

  return (
    <div className="h-full space-y-3 overflow-y-auto px-5 py-4">
      {onDecideGate && isGateWaiting && (
        <ActionRow
          icon={<ShieldCheck className="h-4 w-4" />}
          tone="amber"
          title="Decide gate"
          desc="Open the full-screen review to approve, redirect, or cancel."
          buttonLabel="Decide"
          onClick={onDecideGate}
        />
      )}

      {onRetry && isFailed && (
        <>
          <ActionRow
            icon={<RefreshCw className="h-4 w-4" />}
            tone="ruby"
            title="Retry node"
            desc={
              lastFailed?.applied_rule
                ? `Re-run from scratch. Last failure (${classLabel(lastFailed.error_class!)}) was handled by ${lastFailed.applied_rule}.`
                : 'Re-run this node from scratch with the current harness/model.'
            }
            buttonLabel="Retry"
            onClick={onRetry}
            disabled={guarded}
            disabledReason={guardMsg}
          />
          {overrides && (
            <div className="rounded-xl border border-white/5 bg-black/20 p-3.5">
              <div className="mb-2.5 text-[10px] font-bold uppercase tracking-widest text-slate-500">
                Retry with
              </div>
              <RerunOptions overrides={overrides} />
            </div>
          )}
        </>
      )}

      {onReplay && !graphless && (
        <ActionRow
          icon={<RotateCcw className="h-4 w-4" />}
          tone="cyan"
          title="Replay from node"
          desc="Re-execute this node and everything downstream. The affected nodes are ringed on the graph before you confirm."
          buttonLabel="Replay…"
          onClick={onReplay}
        />
      )}

      {onStop && isRunning && (
        <ActionRow
          icon={<XCircle className="h-4 w-4" />}
          tone="ruby"
          title="Stop node"
          desc="Cancel the in-flight execution."
          buttonLabel="Stop"
          onClick={onStop}
        />
      )}

      {guarded && (
        <div className="flex items-start gap-2 rounded-lg border border-amber-500/20 bg-amber-950/10 p-3 text-xs text-amber-300/90">
          <AlertCircle className="mt-px h-4 w-4 shrink-0 text-amber-400" />
          <span>{guardMsg}</span>
        </div>
      )}
    </div>
  );
}

const ACTION_TONE: Record<string, string> = {
  ruby: 'border-rose-500/20 bg-rose-950/10',
  cyan: 'border-cyan-500/20 bg-cyan-950/10',
  amber: 'border-amber-500/20 bg-amber-950/10',
};
const ACTION_BTN: Record<string, string> = {
  ruby: 'bg-rose-600 hover:bg-rose-500 text-white',
  cyan: 'bg-cyan-600 hover:bg-cyan-500 text-white',
  amber: 'bg-amber-500 hover:bg-amber-600 text-black',
};

function ActionRow({
  icon,
  tone,
  title,
  desc,
  buttonLabel,
  onClick,
  disabled,
  disabledReason,
}: {
  icon: React.ReactNode;
  tone: 'ruby' | 'cyan' | 'amber';
  title: string;
  desc: string;
  buttonLabel: string;
  onClick: () => void;
  disabled?: boolean;
  disabledReason?: string;
}) {
  return (
    <div className={`flex items-center gap-3 rounded-xl border p-3.5 ${ACTION_TONE[tone]}`}>
      <div className={`shrink-0 ${TONE_TEXT[tone as keyof typeof TONE_TEXT] ?? 'text-slate-400'}`}>
        {icon}
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-xs font-bold uppercase tracking-wider text-slate-200">{title}</div>
        <div className="mt-0.5 text-[11px] leading-relaxed text-slate-400">{desc}</div>
      </div>
      <button
        onClick={onClick}
        disabled={disabled}
        title={disabled ? disabledReason : undefined}
        className={`shrink-0 rounded-lg px-3 py-1.5 text-xs font-bold transition disabled:cursor-not-allowed disabled:bg-slate-700/40 disabled:text-slate-500 ${ACTION_BTN[tone]}`}
      >
        {buttonLabel}
      </button>
    </div>
  );
}

export default ActionsTab;
