import { describe, expect, it } from 'vitest';

import {
  HEADER_ICONS_BELOW_PX,
  HEADER_LABELS_AT_PX,
  nextHeaderDensity,
} from './headerLayout';

const bandMidpoint = (HEADER_ICONS_BELOW_PX + HEADER_LABELS_AT_PX) / 2;

/** `src-tauri/tauri.conf.json`'s default window width. */
const DEFAULT_WINDOW_PX = 1440;

describe('nextHeaderDensity', () => {
  it('leaves the icons threshold strictly below the labels threshold', () => {
    expect(HEADER_ICONS_BELOW_PX).toBeLessThan(HEADER_LABELS_AT_PX);
  });

  // The header showed all four labels at every reachable width before this
  // ladder existed, so a threshold pair that puts the app's own default window
  // on the icons side is a regression, not a tier.
  it('labels the nav at the default window width, from either state', () => {
    expect(nextHeaderDensity(DEFAULT_WINDOW_PX, 'icons')).toBe('labels');
    expect(nextHeaderDensity(DEFAULT_WINDOW_PX, 'labels')).toBe('labels');
  });

  // The band holds `labels` across a resize drag, so its lower edge is the
  // narrowest width the labelled cluster is ever asked to fit at. Measured in
  // WebKitGTK, it fits from 1382px up.
  it('keeps the whole band above the measured 1382px fit point', () => {
    expect(HEADER_ICONS_BELOW_PX).toBeGreaterThan(1382);
  });

  it('shows labels at or above the labels threshold, from either state', () => {
    expect(nextHeaderDensity(HEADER_LABELS_AT_PX, 'icons')).toBe('labels');
    expect(nextHeaderDensity(HEADER_LABELS_AT_PX, 'labels')).toBe('labels');
    expect(nextHeaderDensity(2560, 'icons')).toBe('labels');
  });

  it('drops to icons strictly below the icons threshold, from either state', () => {
    expect(nextHeaderDensity(HEADER_ICONS_BELOW_PX - 1, 'labels')).toBe('icons');
    expect(nextHeaderDensity(HEADER_ICONS_BELOW_PX - 1, 'icons')).toBe('icons');
    expect(nextHeaderDensity(1024, 'labels')).toBe('icons');
  });

  it('keeps the incoming density anywhere inside the band', () => {
    expect(nextHeaderDensity(bandMidpoint, 'labels')).toBe('labels');
    expect(nextHeaderDensity(bandMidpoint, 'icons')).toBe('icons');
    expect(nextHeaderDensity(HEADER_ICONS_BELOW_PX, 'labels')).toBe('labels');
    expect(nextHeaderDensity(HEADER_LABELS_AT_PX - 1, 'icons')).toBe('icons');
  });

  it('keeps the incoming density for an unlaid-out element', () => {
    expect(nextHeaderDensity(0, 'labels')).toBe('labels');
    expect(nextHeaderDensity(0, 'icons')).toBe('icons');
    expect(nextHeaderDensity(-1, 'labels')).toBe('labels');
  });
});
