import { describe, expect, it } from 'vitest';

import {
  DEFAULT_DENSITY,
  densityClasses,
  isDensity,
  type Density,
  type DensityClasses,
} from './density';

const KNOBS: Array<keyof DensityClasses> = ['list', 'card', 'title', 'metrics'];

describe('densityClasses', () => {
  it('keeps the comfortable timeline exactly as the cards render today', () => {
    expect(densityClasses('comfortable')).toEqual({
      list: 'space-y-6',
      card: 'p-5',
      title: 'text-sm',
      metrics: 'text-xs',
    });
  });

  it('tightens every knob for compact', () => {
    const comfortable = densityClasses('comfortable');
    const compact = densityClasses('compact');

    for (const knob of KNOBS) {
      expect(compact[knob], knob).not.toBe(comfortable[knob]);
    }
  });

  it('returns one class per knob, so a value can go straight into a className', () => {
    for (const density of ['comfortable', 'compact'] as Density[]) {
      for (const knob of KNOBS) {
        expect(densityClasses(density)[knob], `${density}.${knob}`).toMatch(/^\S+$/);
      }
    }
  });

  // The card is memoized and takes these as props, so a fresh object per call
  // would re-render every row on every parent render (§4.6).
  it('answers with one stable object per density', () => {
    expect(densityClasses('compact')).toBe(densityClasses('compact'));
  });

  it('opens comfortable', () => {
    expect(DEFAULT_DENSITY).toBe('comfortable');
    expect(densityClasses(DEFAULT_DENSITY)).toBe(densityClasses('comfortable'));
  });
});

describe('isDensity', () => {
  it('accepts the two densities', () => {
    expect(isDensity('comfortable')).toBe(true);
    expect(isDensity('compact')).toBe(true);
  });

  it('rejects anything a persisted value could otherwise smuggle in', () => {
    for (const value of ['', 'cozy', 'COMPACT', null, undefined, 0, {}]) {
      expect(isDensity(value), String(value)).toBe(false);
    }
  });
});
