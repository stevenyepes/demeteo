import type { FeatureOrigin } from '../types';

/**
 * What the origin picker's two controls hold. `null` in either is the
 * *unstated* answer, and the two nulls do not mean the same thing: an unstated
 * base is the project's default branch, an unstated diff base is whatever the
 * run starts from.
 */
export interface OriginSelection {
  base: string | null;
  diffBase: string | null;
}

/** The two launch arguments this selection produces. A key is **absent**, not
 *  `undefined`, when the selection does not state it — see
 *  {@link runOriginArgs}. */
export interface RunOriginArgs {
  origin?: FeatureOrigin;
  diffBaseBranch?: string;
}

function named(branch: string | null): string | null {
  const trimmed = branch?.trim() ?? '';
  return trimmed.length > 0 ? trimmed : null;
}

/**
 * Map a selection onto the `start_feature` / `remote_submit_run` arguments.
 *
 * Both keys are omitted rather than sent as `null` for a selection that states
 * nothing, so the launch payload of a user who never opens this picker is the
 * one that shipped before it existed. `FeatureOrigin::DefaultBranch` is the
 * Rust `Default`, and `origin: null` would be a second spelling of it that
 * every later reader has to know is the same thing.
 *
 * A diff base equal to the base is likewise dropped. It is what
 * `domain/diff_base.rs::resolve` answers anyway for a run that named no base,
 * and writing it down is not free: `features.diff_base_branch` outranks the
 * origin there, so a persisted copy of the base survives as a *declaration*
 * that would keep answering after the origin it merely echoed changed.
 */
export function runOriginArgs(selection: OriginSelection): RunOriginArgs {
  const base = named(selection.base);
  const diffBase = named(selection.diffBase);
  const args: RunOriginArgs = {};
  if (base !== null) args.origin = { kind: 'branch', base };
  if (diffBase !== null && diffBase !== base) args.diffBaseBranch = diffBase;
  return args;
}
