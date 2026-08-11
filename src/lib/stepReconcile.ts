import type { StepExecution } from '../types';

/**
 * One comparator per `StepExecution` field, with optionality stripped by `-?`
 * so a field added to the interface makes this object literal a type error.
 * That is the whole reason the comparison is spelled as a record rather than a
 * key list or a deep-equal helper: a field nobody compares is a row that never
 * gets a new identity, which renders as a stale card with no failing test.
 */
type StepFieldEquality = {
  [K in keyof StepExecution]-?: (a: StepExecution[K], b: StepExecution[K]) => boolean;
};

/**
 * `undefined` and `null` are one value here. An absent optional field and an
 * explicit `null` are the same row to every surface that reads it, and the two
 * spellings do vary across the commands feeding this list.
 */
const sameScalar = (a: unknown, b: unknown): boolean => (a ?? null) === (b ?? null);

const samePathList = (a: string[] | undefined, b: string[] | undefined): boolean => {
  if (a === b) return true;
  if (!a || !b || a.length !== b.length) return false;
  return a.every((path, index) => path === b[index]);
};

const STEP_FIELD_EQUALITY: StepFieldEquality = {
  id: sameScalar,
  feature_id: sameScalar,
  step_id: sameScalar,
  step_index: sameScalar,
  step_kind: sameScalar,
  status: sameScalar,
  cost_usd: sameScalar,
  tokens: sameScalar,
  wall_clock_secs: sameScalar,
  artifact_path: sameScalar,
  artifact_paths: samePathList,
  error_message: sameScalar,
  iteration_count: sameScalar,
  created_at: sameScalar,
  updated_at: sameScalar,
  cache_read_input_tokens: sameScalar,
  cache_creation_input_tokens: sameScalar,
};

const STEP_FIELDS = Object.keys(STEP_FIELD_EQUALITY) as (keyof StepExecution)[];

function sameField(key: keyof StepExecution, a: StepExecution, b: StepExecution): boolean {
  const equal = STEP_FIELD_EQUALITY[key] as (x: unknown, y: unknown) => boolean;
  return equal(a[key], b[key]);
}

function sameStep(a: StepExecution, b: StepExecution): boolean {
  return a === b || STEP_FIELDS.every((key) => sameField(key, a, b));
}

/**
 * Merge a freshly fetched step list into the one already in state, preserving
 * object identity wherever the values did not change — and returning `prev`
 * itself when nothing did.
 *
 * Every step row arrives off IPC as a new object, so `setSteps(list)` replaced
 * the array and all of its rows on each reload even when the run was idle. That
 * invalidated every downstream `useMemo` and made `memo` on a row component
 * worthless: the props were never referentially equal. Reconciling by id turns
 * "the backend answered" back into "something changed", which is the question
 * the render path is actually asking.
 *
 * Rows are matched by `id`, so a reorder reuses every row and only genuinely
 * changed rows get a new identity.
 */
export function reconcileSteps(prev: StepExecution[], next: StepExecution[]): StepExecution[] {
  if (prev.length === next.length && next.every((row, index) => sameStep(prev[index], row))) {
    return prev;
  }

  const previousById = new Map(prev.map((row) => [row.id, row]));
  return next.map((row) => {
    const before = previousById.get(row.id);
    return before && sameStep(before, row) ? before : row;
  });
}
