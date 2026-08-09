import { RunEventFeed } from '../../RunEventFeed';
import { formatDuration } from '../../../lib/utils';
import type { RunEvent, StepAttempt } from '../../../types';
import type { NodeRunStatus } from '../types';
import { AttemptTable } from './AttemptTable';
import { SequenceTasks } from './SequenceTasks';
import { formatCost } from './format';

/** Overview: node totals + the per-attempt history table (`step_attempts`),
 *  plus — for a sequence node — its landed-vs-pending task list (P2.5). */
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
  runEvents,
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
  runEvents?: RunEvent[];
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

      {/* Raw run-event log (P1.13). The standalone `RunEventTimeline` is no
          longer a separate surface (P2.6) — its feed lives here, one shape for
          both transports (local push / remote poll). Run-level, not per-node,
          so it's shown whenever there's a feed to read. */}
      {runEvents && runEvents.length > 0 && (
        <div>
          <div className="mb-2 text-[10px] font-bold uppercase tracking-widest text-slate-500">
            Run activity
          </div>
          <div className="max-h-48 space-y-2 overflow-y-auto rounded-xl border border-white/5 bg-[#050608] p-3 font-mono text-[11px]">
            <RunEventFeed events={runEvents} />
          </div>
        </div>
      )}
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
