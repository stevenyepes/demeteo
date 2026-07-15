/**
 * The canonical Demeteo effort ladder, hand-mirrored from
 * `crates/demeteo-core/src/domain/models/effort.rs`.
 *
 * There is no Rust→TS codegen in this repo, so the two sides are kept in
 * lock-step by hand and `effortLevels.test.ts` is the only guard against
 * drift. The string spellings below are exactly what the Rust
 * `#[serde(rename_all = "lowercase")]` emits and accepts on the wire — in
 * particular `XHigh` serialises as `"xhigh"`, not `"x-high"`.
 */
export type EffortLevel = 'low' | 'medium' | 'high' | 'xhigh' | 'max';

/** Every level in ladder order (low → max). Mirrors `EffortLevel::ALL`. */
export const EFFORT_LEVELS: readonly EffortLevel[] = ['low', 'medium', 'high', 'xhigh', 'max'];

/**
 * The terminal fallback of the resolution chain. Mirrors
 * `EffortLevel::DEFAULT` — the one place the "default effort is high"
 * decision lives on the Rust side, so the UI shows the same value when
 * nothing up the chain pins one.
 */
export const DEFAULT_EFFORT: EffortLevel = 'high';

/** Human-facing labels for the picker. */
export const EFFORT_LABELS: Readonly<Record<EffortLevel, string>> = {
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  xhigh: 'Extra high',
  max: 'Max',
};

/**
 * The levels each agent actually accepts per invocation. Mirrors
 * `EffortLevel::supported_for`.
 *
 * The live source of truth for the picker is `AgentCatalogEntry.effort_levels`
 * (straight off the agent's declared Rust capabilities), so a UI that has the
 * catalog should prefer that. This table exists for the pure/offline paths
 * (the workflow editor, which has no machine context to probe) and as the
 * mirror the drift test asserts against.
 */
export const EFFORT_SUPPORT: Readonly<Record<string, readonly EffortLevel[]>> = {
  'claude-code': EFFORT_LEVELS,
  // `max` exists only on some gpt-5.6-* models, so codex does not declare it.
  codex: ['low', 'medium', 'high', 'xhigh'],
  opencode: EFFORT_LEVELS,
  // No per-invocation effort control at all — effort lives in
  // ~/.hermes/config.yaml. Honest degradation: the picker greys out.
  hermes: [],
};

/**
 * The levels `kind` supports. An unknown kind (a newly-registered agent this
 * build has never heard of) falls back to the full ladder rather than to
 * "unsupported" — the Rust adapter clamps anyway, so guessing wide degrades
 * to a clamp, while guessing empty would wrongly grey the control out.
 */
export function supportedEffortsFor(kind: string): readonly EffortLevel[] {
  return EFFORT_SUPPORT[kind] ?? EFFORT_LEVELS;
}

/** Type guard for a value coming off the wire or out of a `<select>`. */
export function isEffortLevel(value: unknown): value is EffortLevel {
  return typeof value === 'string' && (EFFORT_LEVELS as readonly string[]).includes(value);
}

/**
 * Project `level` onto an arbitrary supported set. Mirrors the clamp rule of
 * `EffortLevel::clamp_for`: `null` when the set is empty; otherwise the level
 * itself if supported, else the highest supported level strictly below it,
 * else the lowest supported level. Total by construction — the result is
 * always `null` or a member of `supported`.
 *
 * Takes the set rather than the kind so it can be driven by the live
 * `AgentCatalogEntry.effort_levels` as well as by the static table.
 */
export function clampToSupported(
  supported: readonly EffortLevel[],
  level: EffortLevel,
): EffortLevel | null {
  if (supported.length === 0) return null;
  if (supported.includes(level)) return level;

  const rank = (l: EffortLevel) => EFFORT_LEVELS.indexOf(l);
  const below = supported.filter((l) => rank(l) < rank(level));
  if (below.length > 0) {
    return below.reduce((best, l) => (rank(l) > rank(best) ? l : best));
  }
  return supported.reduce((lowest, l) => (rank(l) < rank(lowest) ? l : lowest));
}

/** [`clampToSupported`] against the static per-agent table. */
export function clampForAgent(kind: string, level: EffortLevel): EffortLevel | null {
  return clampToSupported(supportedEffortsFor(kind), level);
}

/**
 * Reconcile a picker's current effort against a (possibly new) supported set —
 * call this whenever the effective harness changes so the control never keeps
 * showing (and silently re-sending) a level the agent can't run.
 *
 * `''` (inherit) stays `''`. Otherwise the level is clamped down to the nearest
 * supported rung so the displayed value matches what the backend's `clamp_for`
 * will actually emit; an agent with no effort control (empty set) collapses
 * back to `''` (inherit).
 */
export function reconcileEffort(
  effort: EffortLevel | '',
  supported: readonly EffortLevel[],
): EffortLevel | '' {
  if (!effort) return '';
  return clampToSupported(supported, effort) ?? '';
}
