/**
 * How the run column should use the width it was actually given.
 *
 * The run view is one vertical column of blocks — stepper, graph, meta panels,
 * logs — and stacking them is right for every window the app is normally used
 * in. It stops being right only once the column is wide enough that a second
 * track earns its space: below that, splitting buys two narrow columns instead
 * of one readable one.
 *
 * These verdicts are **opening positions, not settlements**. The inspector is a
 * `SplitPane` the user drags, so what this module answers is the state a
 * freshly opened run starts in; a width the user chose outranks every number
 * here. Nothing may re-derive a layout from these and overwrite that choice.
 *
 * The verdicts live here rather than in `FeatureDetail` because they *are*
 * policy decisions — nothing here reads the DOM or measures anything, and each
 * is answerable from a test with two numbers. The component's job is only to
 * measure the column and pass the numbers in.
 */

import {
  DEFAULT_MIN_PRIMARY,
  DEFAULT_MIN_SECONDARY,
  resolveSecondaryWidth,
} from './ui/splitPaneGeometry';

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
 * Pick the meta column's opening layout for the space the run column has.
 *
 * `'split'` requires a real measurement: a non-null size, `width` at or above
 * `SPLIT_MIN_WIDTH`, and a positive height. Everything else — nothing measured
 * yet, a zero or negative width, a collapsed height — answers `'stacked'`,
 * which is the layout that works at every size. A hidden or not-yet-laid-out
 * column reports zeros, and that must not read as "wide".
 *
 * This decides where the run opens, not where it stays: the meta track shares
 * the column with a draggable inspector, so re-asking on every measurement and
 * re-applying the answer would fight the user's drag. Ask once per run.
 */
export function pickRunLayout(size: RunColumnSize | null): RunLayoutMode {
  if (!size) return 'stacked';
  if (size.height <= 0) return 'stacked';
  return size.width >= SPLIT_MIN_WIDTH ? 'split' : 'stacked';
}

/**
 * Where the step inspector sits: beside the run surface, or beneath it.
 *
 * The inspector is always present — it clamps to a minimum width and never
 * collapses to zero (plan §7, settled 2026-08-08). That decision leaves one
 * case it does not answer: a column too narrow to seat `minPrimary +
 * minSecondary` has no width to give a second track, and forcing one there
 * wedges both panes at their minimums with the run surface unusable.
 * `'stacked'` is the answer, and it keeps the settled decision intact — the
 * inspector moves below the run surface, still always on screen, never hidden
 * and never behind an affordance the user has to find.
 */
export type InspectorLayoutMode = 'side' | 'stacked';

/**
 * Narrowest run column that can seat the inspector beside the run surface.
 *
 * Derived rather than chosen: it is exactly the two pane minimums the
 * `SplitPane` divider already clamps against, so the threshold cannot drift
 * away from the geometry that enforces it.
 */
export const INSPECTOR_SIDE_MIN_WIDTH = DEFAULT_MIN_PRIMARY + DEFAULT_MIN_SECONDARY;

/** Same measurement discipline as {@link pickRunLayout}: zeros are not a width. */
export function pickInspectorLayout(size: RunColumnSize | null): InspectorLayoutMode {
  if (!size) return 'stacked';
  if (size.height <= 0) return 'stacked';
  return size.width >= INSPECTOR_SIDE_MIN_WIDTH ? 'side' : 'stacked';
}

/**
 * Share of the run column the inspector opens at, before any drag.
 *
 * A third, because the run surface is the subject and the inspector annotates
 * it; half would invert that on every column the app is actually used in. The
 * fraction only does work in the middle of the range, and the ends say why it
 * is this one: at the `INSPECTOR_SIDE_MIN_WIDTH` floor a third is below
 * `DEFAULT_MIN_SECONDARY` and the clamp decides instead, so a smaller fraction
 * would change nothing there; at 1440 px it opens at 480 px, which holds the
 * inspector's tab row without wrapping while leaving the graph its ELK layout
 * width. Above that the graph is the block that keeps benefiting from
 * width, which is the argument against a larger share, not for one.
 */
export const INSPECTOR_WIDTH_FRACTION = 1 / 3;

/**
 * Initial secondary-pane width for a run column opening on `'side'`.
 *
 * Clamped through `resolveSecondaryWidth` rather than by arithmetic here, so
 * the opening width is reachable by the divider that has to honour it — a
 * default outside the drag range is a width the user can never return to.
 */
export function defaultInspectorWidth(size: RunColumnSize | null): number {
  if (!size || size.width <= 0) return DEFAULT_MIN_SECONDARY;
  return resolveSecondaryWidth(size.width * INSPECTOR_WIDTH_FRACTION, {
    containerWidth: size.width,
    minPrimary: DEFAULT_MIN_PRIMARY,
    minSecondary: DEFAULT_MIN_SECONDARY,
  });
}
