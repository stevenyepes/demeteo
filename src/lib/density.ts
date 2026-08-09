/**
 * Comfortable / compact for the two long lists — the step timeline and the
 * project view's pipeline list (`docs/UI_REDESIGN_PLAN.md` §3.7): a padding and
 * font-size swap, never a second set of components.
 *
 * The classes are spelled here rather than in the cards so the mapping is one
 * table a test can read, and so the two densities cannot drift apart knob by
 * knob. Every string below has to be a class Tailwind actually emits a rule
 * for — `scripts/check-classes.mjs` cannot see these, because it only reads
 * literals sitting in a `class`/`className` attribute and these arrive through
 * a prop. Tailwind's scanner does see them (`@source` covers `src/lib`), so a
 * name that is wrong here is a rule that silently does not exist, with nothing
 * in the toolchain to say so. Verify a change against the compiled stylesheet's
 * *selectors*, the way that script does.
 *
 * Two tables rather than one generalized one: the surfaces size different
 * elements, and the alternatives are both worse. A record holding the union of
 * the knobs lets a row read a knob meant for the other surface and render
 * something plausible; a shared set of knobs ties the timeline's proportions to
 * the project view's for no reason beyond both being lists. `Density`,
 * `DEFAULT_DENSITY` and `isDensity` stay shared — that part *is* one decision,
 * and it is the one Phase 6 persists.
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

/** The knobs the pipeline list turns, one per tier the row renders. */
export interface PipelineDensityClasses {
  /** Vertical rhythm between pipeline rows. */
  list: string;
  /** Padding inside one pipeline card. */
  card: string;
  /** The feature's title, the loudest thing on the row. */
  title: string;
  /** The scan tier's elapsed value, which stays a step above the tiers below. */
  elapsed: string;
  /** The context and detail tiers, which read as one weight. */
  meta: string;
}

export const DEFAULT_DENSITY: Density = 'comfortable';

const CLASSES: Record<Density, DensityClasses> = {
  comfortable: { list: 'space-y-6', card: 'p-5', title: 'text-sm', metrics: 'text-xs' },
  compact: { list: 'space-y-2', card: 'p-3', title: 'text-xs', metrics: 'text-[10px]' },
};

const PIPELINE_CLASSES: Record<Density, PipelineDensityClasses> = {
  comfortable: {
    list: 'space-y-4', card: 'p-5', title: 'text-lg', elapsed: 'text-sm', meta: 'text-xs',
  },
  compact: {
    list: 'space-y-2', card: 'p-3', title: 'text-sm', elapsed: 'text-xs', meta: 'text-[10px]',
  },
};

export function densityClasses(density: Density): DensityClasses {
  return CLASSES[density];
}

export function pipelineDensityClasses(density: Density): PipelineDensityClasses {
  return PIPELINE_CLASSES[density];
}

/** For a persisted value coming back as an unknown string (Phase 6). */
export function isDensity(value: unknown): value is Density {
  return value === 'comfortable' || value === 'compact';
}
