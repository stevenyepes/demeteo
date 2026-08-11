import { describe, expect, it } from 'vitest';

import {
  DEFAULT_DENSITY,
  densityClasses,
  isDensity,
  pipelineDensityClasses,
  type Density,
  type DensityClasses,
  type PipelineDensityClasses,
} from './density';

const KNOBS: Array<keyof DensityClasses> = ['list', 'card', 'title', 'metrics'];
const PIPELINE_KNOBS: Array<keyof PipelineDensityClasses> = [
  'list', 'card', 'title', 'elapsed', 'meta',
];

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

describe('pipelineDensityClasses', () => {
  it('keeps the comfortable list exactly as the project view renders today', () => {
    expect(pipelineDensityClasses('comfortable')).toEqual({
      list: 'space-y-4',
      card: 'p-5',
      title: 'text-lg',
      elapsed: 'text-sm',
      meta: 'text-xs',
    });
  });

  it('tightens every knob for compact', () => {
    const comfortable = pipelineDensityClasses('comfortable');
    const compact = pipelineDensityClasses('compact');

    for (const knob of PIPELINE_KNOBS) {
      expect(compact[knob], knob).not.toBe(comfortable[knob]);
    }
  });

  it('returns one class per knob, so a value can go straight into a className', () => {
    for (const density of ['comfortable', 'compact'] as Density[]) {
      for (const knob of PIPELINE_KNOBS) {
        expect(pipelineDensityClasses(density)[knob], `${density}.${knob}`).toMatch(/^\S+$/);
      }
    }
  });

  it('answers with one stable object per density', () => {
    expect(pipelineDensityClasses('compact')).toBe(pipelineDensityClasses('compact'));
  });

  // The two lists size different elements, so they are two tables. Sharing one
  // object would tie the timeline's proportions to the project view's for no
  // reason beyond both being lists.
  it('is a table of its own, not the timeline read through another name', () => {
    expect(pipelineDensityClasses('comfortable')).not.toBe(densityClasses('comfortable'));
    expect(pipelineDensityClasses('comfortable').title).not.toBe(densityClasses('comfortable').title);
  });

  it('opens comfortable', () => {
    expect(pipelineDensityClasses(DEFAULT_DENSITY)).toBe(pipelineDensityClasses('comfortable'));
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
