/**
 * How the run column should use the width it was actually given.
 *
 * The run view is one vertical column of blocks — stepper, graph, meta panels,
 * logs — and stacking them is right for every window the app is normally used
 * in. It stops being right only once the column is wide enough that a second
 * track earns its space: below that, splitting buys two narrow columns instead
 * of one readable one.
 *
 * The verdict lives here rather than in `FeatureDetail` because it *is* a
 * policy decision — it reads no DOM, measures nothing, and is answerable from
 * a test with two numbers. The component's job is only to measure the column
 * and pass the numbers in.
 */

export interface RunColumnSize {
  width: number;
  height: number;
}

export type RunLayoutMode = 'stacked' | 'split';

/**
 * Measured run-column width at which the meta panels earn their own track.
 *
 * Note this is the *column* width, not the viewport's: the shell's navigation
 * has already been subtracted by the time it's measured. 1600 px is where a
 * second track stops being a pair of cramped columns — a half-width window on
 * a 4K display leaves the column around 1700 px and splits; a half-width
 * window on a 1440p display leaves around 1280 px and does not. Anything
 * lower would split the common laptop-plus-external-monitor case, which is
 * the case the stacked layout exists for.
 */
export const SPLIT_MIN_WIDTH = 1600;

/**
 * Reading cap for prose-bearing blocks, in `ch`.
 *
 * Prose past roughly this measure is harder to track line-to-line, so
 * descriptions, plans and agent output stay capped. Chrome — tables, the
 * stepper, the graph — is uncapped: those read *worse* when squeezed, and
 * capping them is what left the wide window mostly empty.
 */
export const PROSE_CH = 96;

/**
 * Pick the run column's layout for the space it has.
 *
 * `'split'` requires a real measurement: a non-null size, `width` at or above
 * `SPLIT_MIN_WIDTH`, and a positive height. Everything else — nothing measured
 * yet, a zero or negative width, a collapsed height — answers `'stacked'`,
 * which is the layout that works at every size. A hidden or not-yet-laid-out
 * column reports zeros, and that must not read as "wide".
 */
export function pickRunLayout(size: RunColumnSize | null): RunLayoutMode {
  if (!size) return 'stacked';
  if (size.height <= 0) return 'stacked';
  return size.width >= SPLIT_MIN_WIDTH ? 'split' : 'stacked';
}
