import { describe, expect, it } from 'vitest';

import { NO_CAP, parseDetachedCap } from './detachedCaps';

describe('parseDetachedCap', () => {
  it.each([
    ['cost', ''],
    ['cost', '   '],
    ['wallClock', ''],
    ['wallClock', '  '],
  ] as const)('reads a blank %s cap as no cap', (kind, raw) => {
    expect(parseDetachedCap(raw, kind)).toEqual(NO_CAP);
  });

  it.each(['0', '-5', 'abc', 'NaN', 'Infinity', '-Infinity'])(
    'refuses a cost cap of %s instead of dropping it',
    (raw) => {
      expect(parseDetachedCap(raw, 'cost')).toEqual({
        value: undefined,
        error: 'Enter an amount greater than 0, or leave blank for no cap.',
      });
    },
  );

  it('accepts a fractional cost cap', () => {
    expect(parseDetachedCap(' 1.5 ', 'cost')).toEqual({ value: 1.5, error: null });
  });

  it.each(['0', '-1', '1.5', 'soon', 'Infinity'])(
    'refuses a wall-clock cap of %s instead of dropping it',
    (raw) => {
      expect(parseDetachedCap(raw, 'wallClock')).toEqual({
        value: undefined,
        error: 'Enter a whole number of minutes above 0, or leave blank for no cap.',
      });
    },
  );

  it('accepts a whole-minute wall-clock cap', () => {
    expect(parseDetachedCap('30', 'wallClock')).toEqual({ value: 30, error: null });
  });
});
