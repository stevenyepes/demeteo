import { AlertTriangle, MessageSquare, Square } from 'lucide-react';

import { findPlanIssues } from '../../lib/taskPlan';
import type { PlanKind, PlannedTask, TaskPlan } from '../../types';

interface CycleGroup {
  cycle: number;
  kind: PlanKind;
  tasks: PlannedTask[];
}

const CYCLE_BORDER: Record<PlanKind, string> = {
  greenfield: 'border-violet-500/50',
  rework: 'border-cyan-500/50',
};

/** A `Record` lookup on an agent-written value yields `undefined`, and
 *  `undefined` interpolated into a `className` is the literal string
 *  "undefined" — a class no stylesheet defines, which the browser silently
 *  ignores and every gate in `npm run checks` passes (AGENTS §7). The border
 *  falls back to `currentColor` and nothing anywhere reports it. */
function cycleBorder(kind: PlanKind): string {
  return CYCLE_BORDER[kind] ?? CYCLE_BORDER.greenfield;
}

/**
 * `plan.history` (if present) plus the current cycle, synthesized the same
 * way `SequenceTasks.tsx`'s `groupByCycle` treats a flat task list — see
 * `src/components/canvas/nodePanel/SequenceTasks.tsx:129-141`. Unlike that
 * helper this one has no boundary to detect: the artifact already carries
 * cycles as discrete `PlanCycle` records, so history is concatenated with the
 * synthesized current group in order.
 *
 * Every label is supplied here rather than read: no `task-list.json` on disk
 * carries `cycle`/`kind`/`history` (see `TaskPlan` in `src/types.ts`), so the
 * production shape is exactly the one that would default. Tasks arrays are
 * re-checked because this component is also handed plans that bypassed
 * `isTaskPlan` entirely.
 */
function buildCycleGroups(plan: TaskPlan): CycleGroup[] {
  const history = Array.isArray(plan.history) ? plan.history : [];
  return [...history, plan].map((cycle) => ({
    cycle: cycle.cycle ?? 0,
    kind: cycle.kind ?? 'greenfield',
    tasks: Array.isArray(cycle.tasks) ? cycle.tasks : [],
  }));
}

/**
 * Renders a `task-list.json` artifact as cards — the declarative plan, not
 * `SequenceState`'s execution view (`SequenceTasks.tsx`). No landed/running/
 * failed glyph belongs here; the only status a card carries is its cycle's
 * `kind`, painted as a static left border.
 */
export function TaskListArtifact({ plan }: { plan: TaskPlan }) {
  const groups = buildCycleGroups(plan);
  const labelled = groups.length > 1;
  const issues = findPlanIssues(plan);
  const notes = typeof plan.notes === 'string' ? plan.notes.trim() : '';
  const empty = groups.every((group) => group.tasks.length === 0) && !notes;

  return (
    <div className="space-y-4">
      {notes && (
        // Load-bearing for a rework cycle with no tasks: without this, a
        // decomposition that decided no ticket was warranted renders as a
        // bare empty state instead of explaining why.
        <div className="glass-panel flex items-start gap-2.5 rounded-xl border border-cyan-500/20 bg-cyan-500/5 p-3 text-xs text-cyan-200">
          <MessageSquare className="mt-0.5 h-3.5 w-3.5 shrink-0 text-cyan-400" />
          <p className="leading-relaxed">{notes}</p>
        </div>
      )}

      {issues.length > 0 && (
        <div className="glass-panel rounded-xl border border-amber-500/20 bg-amber-500/5 p-3 text-xs text-amber-200">
          <div className="mb-1.5 flex items-center gap-2 font-bold uppercase tracking-wider text-amber-400">
            <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
            Plan issues
          </div>
          <ul className="list-disc space-y-1 pl-5">
            {issues.map((issue, i) => (
              <li key={i}>{issue}</li>
            ))}
          </ul>
        </div>
      )}

      {/* A plan with no tickets and nothing to say about it renders no cards
          and — as the single group — no label either, so the panel body was
          blank. Blank is indistinguishable from a broken viewer, and this one
          is read at a gate where the reviewer's next click is Approve. */}
      {empty && (
        <div className="glass-panel rounded-xl border border-white/5 p-6 text-center text-xs text-slate-400">
          This decomposition produced no tickets, and left no note explaining why.
        </div>
      )}

      {groups.map((group, groupIndex) => (
        <div key={`${groupIndex}-${group.cycle}`} className="space-y-3">
          {labelled && (
            <div className="flex items-baseline justify-between px-1">
              <span className="text-[10px] font-bold uppercase tracking-widest text-violet-400/70">
                {group.cycle === 0 ? 'Original decomposition' : `Rework ${group.cycle}`}
              </span>
              <span className="font-mono text-[10px] text-slate-500">
                {group.tasks.length} {group.tasks.length === 1 ? 'ticket' : 'tickets'}
              </span>
            </div>
          )}
          {group.tasks.map((task, i) => (
            // Index, not just id, in the key: findPlanIssues below flags a
            // duplicate task id as non-blocking, so two cards can legitimately
            // share one and must not collide as React children.
            <TaskCard key={`${groupIndex}-${i}-${task.id}`} index={i + 1} kind={group.kind} task={task} />
          ))}
        </div>
      ))}
    </div>
  );
}

function TaskCard({ index, kind, task }: { index: number; kind: PlanKind; task: PlannedTask }) {
  return (
    <div className={`glass-panel border-l-2 p-4 ${cycleBorder(kind)}`}>
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-baseline gap-2">
          <span className="shrink-0 font-mono text-[10px] text-slate-500">{index}</span>
          <h3 className="truncate text-sm font-semibold text-slate-100" title={task.title}>
            {task.title}
          </h3>
        </div>
        <span className="shrink-0 font-mono text-[10px] text-slate-500" title={task.id}>
          {task.id}
        </span>
      </div>

      <p className="mt-2 text-xs leading-relaxed text-slate-300">{task.description}</p>

      {Array.isArray(task.files) && task.files.length > 0 && (
        <div className="mt-3 flex flex-wrap gap-1.5">
          {task.files.map((file, i) => (
            <span
              key={`${i}-${file}`}
              className="rounded-md border border-white/10 bg-white/5 px-1.5 py-0.5 font-mono text-[10px] text-slate-300"
            >
              {file}
            </span>
          ))}
        </div>
      )}

      {Array.isArray(task.acceptance) && task.acceptance.length > 0 && (
        <ul className="mt-3 space-y-1">
          {task.acceptance.map((item, i) => (
            <li key={i} className="flex items-start gap-2 text-xs text-slate-300">
              <Square className="mt-0.5 h-3 w-3 shrink-0 text-slate-600" />
              <span>{item}</span>
            </li>
          ))}
        </ul>
      )}

      {Array.isArray(task.blocked_by) && task.blocked_by.length > 0 && (
        <div className="mt-3 flex flex-wrap items-center gap-1.5 text-[10px] text-slate-500">
          <span className="font-bold uppercase tracking-wider">Blocked by</span>
          {/* Indexed for the same reason the task cards are: these lists are
              agent-written and nothing rejects a repeat, so `blocked_by:
              ["t1","t1"]` collides as two children under one key. */}
          {task.blocked_by.map((id, i) => (
            <span
              key={`${i}-${id}`}
              className="rounded border border-white/10 bg-white/5 px-1.5 py-0.5 font-mono text-slate-300"
            >
              {id}
            </span>
          ))}
        </div>
      )}

      {task.test_command && (
        <pre className="mt-3 overflow-x-auto rounded-lg border border-white/5 bg-black/40 px-2.5 py-1.5 font-mono text-[11px] text-emerald-300/90">
          {task.test_command}
        </pre>
      )}
    </div>
  );
}

export default TaskListArtifact;
