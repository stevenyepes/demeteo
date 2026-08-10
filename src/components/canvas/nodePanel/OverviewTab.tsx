import { formatDuration } from '../../../lib/utils';
import type { StepAttempt } from '../../../types';
import type { NodeRunStatus } from '../types';
import { AttemptTable } from './AttemptTable';
import { SequenceTasks } from './SequenceTasks';
import { formatCost } from './format';

/** Overview: node totals + the per-attempt history table (`step_attempts`),
 *  plus — for a sequence node — its landed-vs-pending task list (P2.5).
 *
 *  Everything here is scoped to the node. The unified run-event feed used to
 *  hang off the bottom of this tab and was the exception — a run-level log
 *  under a node-level heading, shown "whenever there's a feed to read"; it is
 *  now the run's own `ActivityPanel` (UI_REDESIGN_PLAN §1 D). */
export function OverviewTab({
  run,
  hasExecution,
  attempts,
  loading,
  error,
  isSequence,
  featureId,
  nodeId,
  stepExecutionId,
  version,
}: {
  run: NodeRunStatus | null;
  hasExecution: boolean;
  attempts: StepAttempt[];
  loading: boolean;
  error: string | null;
  isSequence: boolean;
  featureId: string;
  nodeId: string;
  stepExecutionId: string | null;
  version: string;
}) {
  return (
    <div className="h-full space-y-5 overflow-y-auto px-5 py-4">
      <div className="grid grid-cols-3 gap-3">
        <Stat label="Attempts" value={run ? String(Math.max(attempts.length, 1)) : '—'} />
        <Stat label="Total cost" value={formatCost(run?.costUsd)} />
        <Stat
          label="Duration"
          value={run?.wallClockSecs != null ? formatDuration(run.wallClockSecs) : '—'}
        />
      </div>

      {/* The Decision-13 landed prefix, made legible. */}
      {isSequence && (
        <SequenceTasks
          featureId={featureId}
          nodeId={nodeId}
          stepExecutionId={stepExecutionId}
          version={version}
        />
      )}

      <AttemptTable
        hasExecution={hasExecution}
        attempts={attempts}
        loading={loading}
        error={error}
      />
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-white/5 bg-white/[0.02] px-3 py-2">
      <div className="text-[9px] font-bold uppercase tracking-widest text-slate-500">{label}</div>
      <div className="mt-0.5 font-mono text-sm text-slate-200">{value}</div>
    </div>
  );
}

export default OverviewTab;
