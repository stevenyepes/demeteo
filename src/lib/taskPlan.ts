import type { PlannedTask, TaskPlan } from '../types';

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === 'string');
}

function isOptionalStringArray(value: unknown): value is string[] | undefined {
  return value === undefined || isStringArray(value);
}

function isOptionalNullableString(value: unknown): value is string | null | undefined {
  return value === undefined || value === null || typeof value === 'string';
}

function isPlannedTask(value: unknown): value is PlannedTask {
  if (typeof value !== 'object' || value === null) return false;
  const task = value as Record<string, unknown>;
  return (
    typeof task.id === 'string' &&
    typeof task.title === 'string' &&
    typeof task.description === 'string' &&
    isOptionalStringArray(task.files) &&
    isOptionalStringArray(task.acceptance) &&
    isOptionalStringArray(task.blocked_by) &&
    isOptionalNullableString(task.test_command) &&
    isOptionalNullableString(task.retry_note)
  );
}

/** Structural check for the `task-list.json` artifact shape. Only `tasks`
 *  (and each task's `id`/`title`/`description`) is required — `kind`/`cycle`/
 *  other `TaskPlan` fields are agent-written and not guaranteed present, per
 *  the mirror comment on `TaskPlan` in `src/types.ts`. A legacy `subtasks`
 *  payload (the Rust `#[serde(alias = "subtasks")]` shape) has no `tasks`
 *  key and deliberately fails this guard so callers fall back to Monaco. */
export function isTaskPlan(value: unknown): value is TaskPlan {
  if (typeof value !== 'object' || value === null) return false;
  const plan = value as Record<string, unknown>;
  return Array.isArray(plan.tasks) && plan.tasks.every(isPlannedTask);
}

export function parseTaskPlan(raw: string): TaskPlan | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  return isTaskPlan(parsed) ? parsed : null;
}

/** Best-effort, non-blocking lint over an already-parsed `TaskPlan` — not a
 *  port of the Rust `validate_task_plan` (`tasks.rs:260-302`); see spec Open
 *  Questions §7.4. Returns one human-readable string per problem found. */
export function findPlanIssues(plan: TaskPlan): string[] {
  const issues: string[] = [];
  const seenIds = new Set<string>();

  for (const task of plan.tasks) {
    if (seenIds.has(task.id)) {
      issues.push(`Duplicate task id: ${task.id}`);
    }
    seenIds.add(task.id);

    if (task.blocked_by?.includes(task.id)) {
      issues.push(`Task ${task.id} is blocked by itself`);
    }
  }

  return issues;
}
