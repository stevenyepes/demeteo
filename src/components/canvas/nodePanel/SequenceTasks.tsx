import { Fragment, useEffect, useState } from 'react';
import { AlertCircle, Check, CircleDashed, Loader2, XCircle } from 'lucide-react';

import { formatError } from '../../../lib/errors';
import { getSequenceState } from '../../../lib/features';
import { runStatusMeta, TONE_CHIP } from '../../../lib/runStatus';
import type { SequenceState } from '../../../types';
import { EmptyHint } from './EmptyHint';
import { formatCost } from './format';

/**
 * A sequence node's task list, fetched from `sequence_tasks_list` (P2.5).
 *
 * The point is Decision 13's *landed prefix*: tasks whose commit is already on
 * the feature branch (`landed`) are the work a crash-resume or targeted retry
 * will not re-run. They render solid with a filled check and an emerald rail;
 * pending tasks dim; the live/failed task takes its run-status tone — so the
 * split the engine has always tracked is legible for the first time.
 */
export function SequenceTasks({
  featureId,
  nodeId,
  stepExecutionId,
  version,
}: {
  featureId: string;
  nodeId: string;
  stepExecutionId: string | null;
  version: string;
}) {
  const [state, setState] = useState<SequenceState | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!stepExecutionId) {
      setState(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    getSequenceState({ featureId, nodeId, executionId: stepExecutionId })
      .then((s) => {
        if (!cancelled) setState(s);
      })
      .catch((err) => {
        if (!cancelled) setError(formatError(err) || 'Failed to load task list.');
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // Refetch as the node advances (a task landing changes the split).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [featureId, nodeId, stepExecutionId, version]);

  const tasks = state?.tasks ?? [];
  const landedCount = tasks.filter((t) => t.landed).length;

  // Nothing to show until the node has planned. Stay silent rather than render
  // an empty box — a sequence node that hasn't reached its plan is the norm.
  if (!stepExecutionId || (!loading && state && !state.planned)) return null;

  return (
    <div>
      <div className="mb-2 flex items-center justify-between">
        <span className="text-[10px] font-bold uppercase tracking-widest text-slate-500">
          Task list
        </span>
        {tasks.length > 0 && (
          <span className="font-mono text-[10px] text-emerald-400/80">
            {landedCount}/{tasks.length} landed
          </span>
        )}
      </div>

      {loading && !state ? (
        <div className="flex items-center gap-2 py-6 text-xs text-slate-500">
          <Loader2 className="h-4 w-4 animate-spin text-violet-400" /> Loading task list…
        </div>
      ) : error ? (
        <div className="flex items-start gap-2 rounded-lg border border-rose-500/20 bg-rose-950/20 p-3 text-xs text-rose-300">
          <AlertCircle className="mt-px h-4 w-4 shrink-0 text-rose-400" />
          <span>{error}</span>
        </div>
      ) : tasks.length === 0 ? (
        <EmptyHint>No tasks in this node&apos;s plan.</EmptyHint>
      ) : (
        <ol className="space-y-1.5">
          {groupByCycle(tasks).map((group) => (
            <Fragment key={group.cycle}>
              {/* Only labelled once a rework cycle exists — a single-cycle
                  node is the norm and a "Cycle 0" header on it is noise. */}
              {group.labelled && (
                <li className="flex items-baseline justify-between px-1 pb-0.5 pt-2 first:pt-0">
                  <span className="text-[10px] font-bold uppercase tracking-widest text-violet-400/70">
                    {group.cycle === 0 ? 'Original decomposition' : `Rework ${group.cycle}`}
                  </span>
                  <span className="font-mono text-[10px] text-slate-500">
                    {group.tasks.length} {group.tasks.length === 1 ? 'ticket' : 'tickets'}
                  </span>
                </li>
              )}
              {group.tasks.map((t, i) => (
                <SequenceTaskRow key={`${group.cycle}-${t.id}`} index={i + 1} task={t} />
              ))}
            </Fragment>
          ))}
        </ol>
      )}
    </div>
  );
}

/**
 * Split a flat task list into its decomposition cycles, in order.
 *
 * A step that a downstream verdict sent back has planned more than one list:
 * the original decomposition, then one delta per rework cycle. Both are on the
 * branch, so both are shown — rendering only the list that ran last would
 * present a four-ticket delta as if it were the whole feature.
 *
 * `labelled` is false for the common single-cycle node, where a header would
 * name a distinction that isn't there yet.
 */
function groupByCycle(
  tasks: SequenceState['tasks'],
): { cycle: number; tasks: SequenceState['tasks']; labelled: boolean }[] {
  const groups: { cycle: number; tasks: SequenceState['tasks']; labelled: boolean }[] = [];
  for (const task of tasks) {
    const cycle = task.cycle ?? 0;
    const last = groups[groups.length - 1];
    if (last && last.cycle === cycle) last.tasks.push(task);
    else groups.push({ cycle, tasks: [task], labelled: false });
  }
  const multi = groups.length > 1;
  return groups.map((g) => ({ ...g, labelled: multi }));
}

function SequenceTaskRow({ index, task }: { index: number; task: SequenceState['tasks'][number] }) {
  const meta = runStatusMeta(task.landed ? 'completed' : task.status);
  const isFailed = task.status === 'failed' || task.status === 'interrupted';
  const isActive = task.status === 'running';

  return (
    <li
      className={`flex items-start gap-3 rounded-lg border-l-2 py-2 pl-3 pr-2 ${
        task.landed
          ? 'border-emerald-500/60 bg-emerald-500/[0.04]'
          : isFailed
            ? 'border-rose-500/50 bg-rose-500/[0.04]'
            : isActive
              ? 'border-cyan-500/50 bg-cyan-500/[0.04]'
              : 'border-white/5 bg-white/[0.01] opacity-70'
      }`}
    >
      {/* Landed/pending glyph */}
      <span className="mt-0.5 shrink-0">
        {task.landed ? (
          <Check className="h-3.5 w-3.5 text-emerald-400" />
        ) : isActive ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin text-cyan-400" />
        ) : isFailed ? (
          <XCircle className="h-3.5 w-3.5 text-rose-400" />
        ) : (
          <CircleDashed className="h-3.5 w-3.5 text-slate-600" />
        )}
      </span>

      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="shrink-0 font-mono text-[10px] text-slate-500">{index}</span>
          <span className="truncate text-xs text-slate-200" title={task.title || task.id}>
            {task.title || task.id}
          </span>
        </div>
        {task.error_message && isFailed && (
          <div className="mt-1 truncate text-[10px] text-rose-300/70" title={task.error_message}>
            {task.error_message}
          </div>
        )}
      </div>

      <div className="flex shrink-0 items-center gap-2">
        {typeof task.cost_usd === 'number' && task.cost_usd > 0 && (
          <span className="font-mono text-[10px] text-emerald-400/80">
            {formatCost(task.cost_usd)}
          </span>
        )}
        <span
          className={`rounded px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider ${TONE_CHIP[meta.tone]}`}
        >
          {task.landed ? 'Landed' : meta.label}
        </span>
      </div>
    </li>
  );
}

export default SequenceTasks;
