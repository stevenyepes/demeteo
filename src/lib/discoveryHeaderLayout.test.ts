import { describe, expect, it } from 'vitest';

import {
  DISCOVERY_HEADER_NARROW_BELOW_PX,
  DISCOVERY_HEADER_WIDE_AT_PX,
  nextDiscoveryHeaderDensity,
} from './discoveryHeaderLayout';

describe('nextDiscoveryHeaderDensity', () => {
  it('keeps the current density before anything is laid out', () => {
    expect(nextDiscoveryHeaderDensity(0, 'wide')).toBe('wide');
    expect(nextDiscoveryHeaderDensity(0, 'narrow')).toBe('narrow');
  });

  it('puts every action inline from the wide threshold up', () => {
    expect(nextDiscoveryHeaderDensity(DISCOVERY_HEADER_WIDE_AT_PX, 'compact')).toBe('wide');
    expect(nextDiscoveryHeaderDensity(1920, 'narrow')).toBe('wide');
  });

  // A half-screen window on a 1920–2560px display leaves the header roughly
  // 1000px: the width the inline action group overflowed at.
  it('renders a split-screen header compact', () => {
    expect(nextDiscoveryHeaderDensity(1000, 'wide')).toBe('compact');
    expect(nextDiscoveryHeaderDensity(DISCOVERY_HEADER_WIDE_AT_PX - 1, 'wide')).toBe('compact');
    expect(nextDiscoveryHeaderDensity(DISCOVERY_HEADER_NARROW_BELOW_PX, 'narrow')).toBe('compact');
  });

  it('renders narrow below the narrow threshold', () => {
    expect(nextDiscoveryHeaderDensity(DISCOVERY_HEADER_NARROW_BELOW_PX - 1, 'compact')).toBe(
      'narrow',
    );
  });
});
