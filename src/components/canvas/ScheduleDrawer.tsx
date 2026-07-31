/**
 * Schedule editing for design mode.
 *
 * The v2 builder replaced `WorkflowEditor`, which was the only caller of
 * `workflow_save_schedule`. Nothing else ever wrote one, so schedules became
 * read-only in the product: `WorkflowList` kept rendering cron / next-run and
 * the backend scheduler kept firing, but an author could no longer change or
 * clear a schedule, nor create one. This restores the write side.
 *
 * It is a **drawer, not a canvas concern**, because decision 41 put scheduling
 * outside the graph: a schedule says when to *start* a run, not what the run
 * does, and it lives on the workflow row rather than in any version. That has
 * two consequences this component leans on:
 *
 * - It saves on its own (`workflow_save_schedule`), not through the builder's
 *   `onSave`. Editing a schedule mints no version row — versions are the
 *   graph's history, and a cron change is not a graph change.
 * - It needs a saved workflow to attach to, so the builder only offers it once
 *   `workflowId` exists.
 *
 * Clearing is explicit: blanking every field and saving sends `null`, which is
 * how the command has always expressed "no schedule".
 */
import { useCallback, useEffect, useState } from 'react';
import { CalendarClock, Loader2, X } from 'lucide-react';

import { useErrorBus } from '../../lib/errorBus';
import { getProjects } from '../../lib/project';
import { saveWorkflowSchedule } from '../../lib/workflows';

export interface WorkflowScheduleValue {
  cron: string;
  title_template: string;
  project_id: string;
  next_run_at?: number | null;
}

export interface ScheduleDrawerProps {
  workflowId: string;
  /** Saved schedule, or `null` when the workflow has none. */
  schedule: WorkflowScheduleValue | null;
  /** A schedule was written; the owner re-reads the workflow row. */
  onSaved: (next: WorkflowScheduleValue | null) => void;
  onClose: () => void;
}

interface ProjectOption {
  id: string;
  name: string;
}

/** A standard 5-field cron expression, which is what the backend's
 *  `calculate_next_run` parses. Checked here so a typo is caught before it
 *  becomes a schedule that silently never fires. */
export function validateSchedule(
  cron: string,
  projectId: string,
): string | null {
  if (!projectId) return 'Pick the project the scheduled runs should start on.';
  if (!cron.trim()) return 'Enter a cron expression, or clear every field to remove the schedule.';
  if (cron.trim().split(/\s+/).length !== 5) {
    return 'A cron expression needs exactly 5 fields: minute hour day-of-month month day-of-week.';
  }
  return null;
}

/** Is the form empty enough to mean "remove the schedule"? */
export function isCleared(
  cron: string,
  titleTemplate: string,
  projectId: string,
): boolean {
  return !cron.trim() && !titleTemplate.trim() && !projectId;
}

export function ScheduleDrawer({
  workflowId,
  schedule,
  onSaved,
  onClose,
}: ScheduleDrawerProps) {
  const { reportError } = useErrorBus();
  const [projects, setProjects] = useState<ProjectOption[]>([]);
  const [cron, setCron] = useState(schedule?.cron ?? '');
  const [titleTemplate, setTitleTemplate] = useState(schedule?.title_template ?? '');
  const [projectId, setProjectId] = useState(schedule?.project_id ?? '');
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let live = true;
    getProjects()
      .then((list) => live && setProjects(list.map((p) => ({ id: p.id, name: p.name }))))
      // The target dropdown stays empty and save is blocked on it, so say so
      // rather than presenting an unusable form.
      .catch((err) => live && reportError(err, { kind: 'internal' }));
    return () => {
      live = false;
    };
  }, [reportError]);

  const save = useCallback(async () => {
    const clearing = isCleared(cron, titleTemplate, projectId);
    if (!clearing) {
      const problem = validateSchedule(cron, projectId);
      if (problem) {
        reportError(problem, { kind: 'validation' });
        return;
      }
    }
    // `next_run_at` is deliberately omitted: the command recomputes it from
    // the cron so a stale one can't pin a schedule to a time that has passed.
    const next: WorkflowScheduleValue | null = clearing
      ? null
      : { cron: cron.trim(), title_template: titleTemplate.trim(), project_id: projectId };

    setSaving(true);
    try {
      await saveWorkflowSchedule(workflowId, next);
      onSaved(next);
      onClose();
    } catch (err) {
      reportError(err, { kind: 'validation' });
    } finally {
      setSaving(false);
    }
  }, [cron, titleTemplate, projectId, workflowId, onSaved, onClose, reportError]);

  return (
    <aside
      className="absolute right-0 top-0 z-20 flex h-full w-[380px] flex-col border-l border-white/5 bg-[#0d0f14] shadow-2xl"
      data-testid="schedule-drawer"
      aria-label="Workflow schedule"
    >
      <header className="flex items-center gap-2 border-b border-white/5 px-4 py-3">
        <CalendarClock className="h-4 w-4 text-violet-400" />
        <h2 className="flex-1 text-sm font-semibold text-slate-100">Schedule</h2>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close schedule"
          className="rounded-lg border border-transparent p-1 text-slate-400 hover:border-slate-700/60 hover:text-white"
        >
          <X className="h-4 w-4" />
        </button>
      </header>

      <div className="flex-1 space-y-4 overflow-y-auto p-4">
        <p className="text-xs leading-relaxed text-slate-400">
          Start a run of this workflow automatically. Clear every field to remove the
          schedule.
        </p>

        <div>
          <label
            htmlFor="schedule-project"
            className="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-slate-400"
          >
            Target project
          </label>
          <select
            id="schedule-project"
            value={projectId}
            onChange={(e) => setProjectId(e.target.value)}
            className="w-full rounded-lg border border-white/10 bg-[#0b0d12] p-2.5 text-sm text-white outline-none focus:border-violet-500"
          >
            <option value="">— Select project —</option>
            {projects.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </div>

        <div>
          <label
            htmlFor="schedule-cron"
            className="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-slate-400"
          >
            Cron expression
          </label>
          <input
            id="schedule-cron"
            type="text"
            value={cron}
            onChange={(e) => setCron(e.target.value)}
            placeholder="0 0 * * *"
            className="w-full rounded-lg border border-white/10 bg-[#0b0d12] p-2.5 font-mono text-sm text-white outline-none focus:border-violet-500"
          />
          <p className="mt-1 text-[11px] text-slate-500">
            minute hour day-of-month month day-of-week
          </p>
        </div>

        <div>
          <label
            htmlFor="schedule-title"
            className="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-slate-400"
          >
            Feature title template
          </label>
          <input
            id="schedule-title"
            type="text"
            value={titleTemplate}
            onChange={(e) => setTitleTemplate(e.target.value)}
            placeholder="Nightly run {{datetime}}"
            className="w-full rounded-lg border border-white/10 bg-[#0b0d12] p-2.5 text-sm text-white outline-none focus:border-violet-500"
          />
        </div>

        {schedule?.next_run_at ? (
          <p className="text-[11px] text-slate-500">
            Next run: {new Date(schedule.next_run_at).toLocaleString()}
          </p>
        ) : null}
      </div>

      <footer className="border-t border-white/5 p-4">
        <button
          type="button"
          onClick={save}
          disabled={saving}
          className="flex w-full items-center justify-center gap-2 rounded-lg bg-violet-600 px-3 py-2 text-sm font-medium text-white transition-colors hover:bg-violet-500 disabled:opacity-50"
        >
          {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
          {isCleared(cron, titleTemplate, projectId) ? 'Remove schedule' : 'Save schedule'}
        </button>
      </footer>
    </aside>
  );
}
