import type { PlanCycle, PlanKind, PlannedTask, TaskPlan } from '../types';

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === 'string');
}

function isOptionalStringArray(value: unknown): value is string[] | undefined {
  return value === undefined || isStringArray(value);
}

function isOptionalNullableString(value: unknown): value is string | null | undefined {
  return value === undefined || value === null || typeof value === 'string';
}

function isOptionalNumber(value: unknown): value is number | undefined {
  return value === undefined || typeof value === 'number';
}

function isOptionalPlanKind(value: unknown): value is PlanKind | undefined {
  return value === undefined || value === 'greenfield' || value === 'rework';
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

function isPlanCycle(value: unknown): value is PlanCycle {
  if (typeof value !== 'object' || value === null) return false;
  const cycle = value as Record<string, unknown>;
  return (
    Array.isArray(cycle.tasks) &&
    cycle.tasks.every(isPlannedTask) &&
    isOptionalNumber(cycle.cycle) &&
    isOptionalPlanKind(cycle.kind)
  );
}

/** Structural check for the `task-list.json` artifact shape. Only `tasks`
 *  (and each task's `id`/`title`/`description`) is required — `kind`/`cycle`/
 *  other `TaskPlan` fields are agent-written and not guaranteed present, per
 *  the mirror comment on `TaskPlan` in `src/types.ts`. A legacy `subtasks`
 *  payload (the Rust `#[serde(alias = "subtasks")]` shape) has no `tasks`
 *  key and deliberately fails this guard so callers fall back to Monaco.
 *
 *  Optional does not mean unchecked. A `true` here licenses a renderer to
 *  dereference every field of the plan, so a *present* field must be the
 *  right type or the whole verdict is a lie: `notes` as an object or
 *  `history` as anything but well-formed cycles reaches the renderer as a
 *  `TypeError`, and with no `ErrorBoundary` anywhere in `src/` that blanks
 *  the window while a run sits parked at the gate. Failing the guard costs
 *  the reviewer a card view and leaves the raw JSON readable in Monaco;
 *  passing a payload the renderer cannot survive costs them the app. */
export function isTaskPlan(value: unknown): value is TaskPlan {
  if (typeof value !== 'object' || value === null) return false;
  const plan = value as Record<string, unknown>;
  return (
    Array.isArray(plan.tasks) &&
    plan.tasks.every(isPlannedTask) &&
    isOptionalNumber(plan.cycle) &&
    isOptionalPlanKind(plan.kind) &&
    isOptionalNullableString(plan.notes) &&
    (plan.history === undefined ||
      (Array.isArray(plan.history) && plan.history.every(isPlanCycle)))
  );
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

/** Non-blocking lint over an already-parsed `TaskPlan`, covering the same
 *  rules as the Rust `validate_task_plan` (`domain/sequence/tasks.rs`) —
 *  blank ids, duplicate ids, and a `blocked_by` naming itself, nothing, or a
 *  task further down the list.
 *
 *  Same rules, opposite disposition, and that is the whole point: the
 *  executor's version returns on the *first* violation and fails the step
 *  non-retryably — after a human approved the gate, at which point only the
 *  producing step can fix it and the run is spent. This one reports every
 *  violation at once, while the reviewer is still looking at the plan and a
 *  redirect is free. It stays advisory rather than disabling Approve because
 *  the reviewer may legitimately know better than the lint; what they may not
 *  do is approve a plan whose defects nobody showed them.
 *
 *  Keep the two in step. A rule that lands there and not here is a plan this
 *  gate renders clean and the next step refuses. Returns one human-readable
 *  string per problem found. */
export function findPlanIssues(plan: TaskPlan): string[] {
  const issues: string[] = [];
  const seenIds = new Set<string>();

  // Same reason `buildCycleGroups` re-checks its arrays: this runs on plans
  // that reached the renderer without passing `isTaskPlan`, where `tasks` may
  // be absent and an `id` may be any JSON scalar. A throw here blanks the
  // window — there is no `ErrorBoundary` in `src/` — at the gate where the
  // reviewer's next click is Approve.
  const tasks = Array.isArray(plan.tasks) ? plan.tasks : [];

  tasks.forEach((task, i) => {
    const id = typeof task.id === 'string' ? task.id.trim() : '';
    if (!id) {
      // Not entered into `seenIds`: two blank ids would otherwise report
      // "Duplicate task id: " with nothing after the colon, in a panel whose
      // job is to mirror the executor's verdict. The Rust `validate_task_plan`
      // returns before inserting (`domain/sequence/tasks.rs`).
      issues.push(`Task at position ${i + 1} has an empty id`);
    } else {
      if (seenIds.has(id)) {
        issues.push(`Duplicate task id: ${id}`);
      }
      seenIds.add(id);
    }

    // `Array.isArray` rather than `?? []`: a scalar `blocked_by` string is a
    // shape this renderer can be handed directly (see the guard's contract
    // above), and iterating one yields a bogus issue per character.
    const blockedBy = Array.isArray(task.blocked_by) ? task.blocked_by : [];
    // A dep repeated in the list is one defect, not one per mention.
    const reported = new Set<string>();
    for (const dep of blockedBy) {
      const target = typeof dep === 'string' ? dep.trim() : '';
      if (!target || reported.has(target)) continue;
      reported.add(target);
      if (target === id) {
        issues.push(`Task ${id} is blocked by itself`);
      } else if (!seenIds.has(target)) {
        // `seenIds` holds exactly the earlier tasks' ids, so "not seen" is
        // both "no such task" and "declared later" — one message, because
        // tasks run in list order and the fix is the same either way.
        issues.push(`Task ${id} is blocked by ${target}, which is not an earlier task in the list`);
      }
    }
  });

  return issues;
}
