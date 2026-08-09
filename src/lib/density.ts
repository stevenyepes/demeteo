/**
 * Comfortable / compact for the step timeline
 * (`docs/UI_REDESIGN_PLAN.md` §3.7): a padding and font-size swap, never a
 * second set of components.
 *
 * The classes are spelled here rather than in the card so the mapping is one
 * table a test can read, and so the two densities cannot drift apart knob by
 * knob. Every string below has to be a class Tailwind actually emits a rule
 * for — `scripts/check-classes.mjs` cannot see these, because it only reads
 * literals sitting in a `class`/`className` attribute and these arrive through
 * a prop. Tailwind's scanner does see them (`@source` covers `src/lib`), so a
 * name that is wrong here is a rule that silently does not exist, with nothing
 * in the toolchain to say so. Verify a change against the compiled stylesheet's
 * *selectors*, the way that script does.
 */

export type Density = 'comfortable' | 'compact';

/** The knobs the timeline turns. One field per element that changes size. */
export interface DensityClasses {
  /** Vertical rhythm between step rows. */
  list: string;
  /** Padding inside one step card. */
  card: string;
  /** The step's name. */
  title: string;
  /** The card's cost / token / duration group. */
  metrics: string;
}

export const DEFAULT_DENSITY: Density = 'comfortable';

const CLASSES: Record<Density, DensityClasses> = {
  comfortable: { list: 'space-y-6', card: 'p-5', title: 'text-sm', metrics: 'text-xs' },
  compact: { list: 'space-y-2', card: 'p-3', title: 'text-xs', metrics: 'text-[10px]' },
};

export function densityClasses(density: Density): DensityClasses {
  return CLASSES[density];
}

/** For a persisted value coming back as an unknown string (Phase 6). */
export function isDensity(value: unknown): value is Density {
  return value === 'comfortable' || value === 'compact';
}
