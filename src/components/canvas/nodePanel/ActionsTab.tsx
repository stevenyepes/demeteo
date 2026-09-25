import { RefreshCw, RotateCcw, ShieldCheck, XCircle } from 'lucide-react';

import { isOutOfBandStep } from '../../../lib/featureSync';
import { isAssignable, takesAssignment } from '../../../lib/stepAssignment';
import type { StepAttempt } from '../../../types';
import { ActionRow } from '../../ui/ActionRow';
import type { HarnessOverrides } from '../../FeatureDetail/useHarnessOverrides';
import type { StepAssignment } from '../../FeatureDetail/useStepAssignment';
import type { NodeConfigV2, NodeRunStatus } from '../types';
import { AssignmentSection } from './AssignmentSection';
import { classLabel } from './format';
import { GuardNote } from './GuardNote';

/** The active ancestor blocking a manual retry/gate decision, if any. */
export interface BlockingAncestor {
  step_id: string;
  status: string;
}

/** Assignment, then retry / replay / stop / decide-gate, with the ancestor
 *  guard. The panel holds no run logic of its own — FeatureDetail owns the
 *  handlers and passes them in, so the canvas and the timeline drive the same
 *  paths.
 *
 *  Assignment leads and stands outside every status branch because it is the
 *  only control here that is not a reaction to something that already
 *  happened: a queued node has no failure to retry and no execution to stop,
 *  and it is exactly the node worth re-pointing. */
export function ActionsTab({
  node,
  run,
  hasActions,
  attempts,
  blockedBy,
  overrides,
  assignment,
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
  /** The harness/model/effort picker the Assignment control reads. Absent
   *  where nobody holds it — the canvas mounts this panel outside the run
   *  view. */
  overrides?: HarnessOverrides;
  /** Writes the picker's trio onto this node. Paired with `overrides`: a
   *  picker with nothing to submit it to is a question with no answer. */
  assignment?: StepAssignment;
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
  // Stop reads the assignability rule from the other side: the spawned process
  // that makes a reassignment unhonourable is the thing there is to stop, so
  // the two can never be asked separately without one of them going stale.
  const isRunning = !isAssignable(status);
  // Keyed on the status alone, not on the node kind. `awaiting_gate` is
  // written by exactly two things — the gate handler, and a non-gate step
  // that parked for a human — and `gate_decide` answers both identically.
  // Requiring `type === 'gate'` left a parked step with no action at all:
  // `isFailed` does not cover `awaiting_gate` either, so the panel offered
  // neither Decide nor Retry and the only way to answer was the transient
  // toast.
  const isGateWaiting = status === 'awaiting_gate';
  const showAssignment =
    overrides !== undefined && assignment !== undefined && !graphless && takesAssignment(node.type);
  const guarded = blockedBy !== null;
  const guardMsg = blockedBy
    ? `Ancestor "${blockedBy.step_id}" is still ${blockedBy.status}. Wait for it to finish.`
    : '';

  // The policy rule the engine applied to this node's most recent failure — the
  // "which rule will apply" hint (P2.4), read straight from the attempt row.
  const lastFailed = [...attempts].reverse().find((a) => a.error_class);

  // Neither rerun reads the picker: `useRerunActions` submits null for agent,
  // model and effort by decision, so both re-run on the node's *stored* pin.
  // The picker sits directly above them and looks like it drives them, so
  // holding the press shut is the only signal a user cannot read past.
  const unapplied = showAssignment && assignment.dirty;
  const unappliedMsg =
    'The Assignment above has unapplied edits. Retry and Replay re-run on the pinned assignment — press Apply to pin your edits first.';

  const anyAction =
    showAssignment ||
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
      {showAssignment && (
        <AssignmentSection
          status={status}
          overrides={overrides}
          assignment={assignment}
          observed={run ?? undefined}
        />
      )}

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
        <ActionRow
          icon={<RefreshCw className="h-4 w-4" />}
          tone="ruby"
          title="Retry node"
          desc={
            lastFailed?.applied_rule
              ? `Re-run from scratch on its current pinned assignment. Last failure (${classLabel(lastFailed.error_class!)}) was handled by ${lastFailed.applied_rule}.`
              : 'Re-run this node from scratch on its current pinned assignment.'
          }
          buttonLabel="Retry"
          onClick={onRetry}
          disabled={guarded || unapplied}
          disabledReason={guarded ? guardMsg : unappliedMsg}
        />
      )}

      {onReplay && !graphless && (
        <ActionRow
          icon={<RotateCcw className="h-4 w-4" />}
          tone="cyan"
          title="Replay from node"
          desc="Re-execute this node and everything downstream on their current pinned assignments. The affected nodes are ringed on the graph before you confirm."
          buttonLabel="Replay…"
          onClick={onReplay}
          disabled={unapplied}
          disabledReason={unappliedMsg}
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

      {unapplied && <GuardNote message={unappliedMsg} />}

      {guarded && <GuardNote message={guardMsg} />}
    </div>
  );
}

export default ActionsTab;
