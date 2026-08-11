import { describe, expect, it } from 'vitest';

import {
  HEADER_COLLAPSE_AT_PX,
  HEADER_EXPAND_BELOW_PX,
  nextHeaderCollapsed,
} from './headerCollapse';

const bandMidpoint = (HEADER_COLLAPSE_AT_PX + HEADER_EXPAND_BELOW_PX) / 2;

describe('nextHeaderCollapsed', () => {
  it('leaves the expand threshold strictly below the collapse threshold', () => {
    expect(HEADER_EXPAND_BELOW_PX).toBeLessThan(HEADER_COLLAPSE_AT_PX);
  });

  it('collapses once the column is scrolled past the collapse threshold', () => {
    expect(nextHeaderCollapsed(HEADER_COLLAPSE_AT_PX, false)).toBe(true);
    expect(nextHeaderCollapsed(HEADER_COLLAPSE_AT_PX + 400, false)).toBe(true);
  });

  it('expands once the column is back within the expand threshold', () => {
    expect(nextHeaderCollapsed(HEADER_EXPAND_BELOW_PX, true)).toBe(false);
    expect(nextHeaderCollapsed(0, true)).toBe(false);
  });

  it('keeps the incoming state anywhere inside the band', () => {
    expect(nextHeaderCollapsed(bandMidpoint, true)).toBe(true);
    expect(nextHeaderCollapsed(bandMidpoint, false)).toBe(false);
    expect(nextHeaderCollapsed(HEADER_COLLAPSE_AT_PX - 1, false)).toBe(false);
    expect(nextHeaderCollapsed(HEADER_EXPAND_BELOW_PX + 1, true)).toBe(true);
  });

  it('reads an elastic overscroll past the top as the top', () => {
    expect(nextHeaderCollapsed(-40, true)).toBe(false);
  });
});
