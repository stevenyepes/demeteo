import { describe, expect, it } from 'vitest';

import {
  anchoredScroll,
  clampZoom,
  fitZoom,
  steppedZoom,
  wheelPixels,
  wheelZoom,
  ZOOM_MAX,
  ZOOM_MIN,
  zoomPercent,
} from './canvasZoom';

describe('clampZoom', () => {
  it('holds the range at both ends', () => {
    expect(clampZoom(9)).toBe(ZOOM_MAX);
    expect(clampZoom(0.01)).toBe(ZOOM_MIN);
    expect(clampZoom(1)).toBe(1);
  });
});

describe('steppedZoom', () => {
  it('scales by the same proportion wherever it starts', () => {
    const low = steppedZoom(0.5, 1) / 0.5;
    const high = steppedZoom(1.5, 1) / 1.5;
    expect(high).toBeCloseTo(low, 3);
  });

  it('returns to where it started after a step each way', () => {
    expect(steppedZoom(steppedZoom(1, 1), -1)).toBeCloseTo(1, 3);
  });
});

describe('wheelZoom', () => {
  it('reads a notch up as zoom in and a notch down as zoom out', () => {
    expect(wheelZoom(1, -100)).toBeGreaterThan(1);
    expect(wheelZoom(1, 100)).toBeLessThan(1);
  });

  it('cancels a notch with its opposite', () => {
    expect(wheelZoom(wheelZoom(1, -100), 100)).toBeCloseTo(1, 3);
  });

  // A line- or page-mode delta read as pixels is a zoom that barely moves:
  // Firefox reports 3 lines where a pixel-mode browser reports ~100px.
  it('converts a line-mode delta to the travel it stands for', () => {
    expect(wheelPixels(3, 1)).toBe(48);
    expect(wheelPixels(1, 2)).toBe(400);
    expect(wheelPixels(120, 0)).toBe(120);
  });
});

describe('fitZoom', () => {
  it('frames on the tighter axis', () => {
    expect(fitZoom({ width: 400, height: 1000 }, { width: 800, height: 800 })).toBeLessThan(
      fitZoom({ width: 1000, height: 1000 }, { width: 800, height: 800 }),
    );
  });

  /**
   * The oscillation this margin exists to prevent: a fit that lands the content
   * exactly on the pane raises the scrollbars, a scrollbar takes width out of
   * the pane's content box, and the `ResizeObserver` refit answers a different
   * number than the one that armed it. Fitting to *less* than the pane means a
   * fitted canvas never overflows, so the second tick agrees with the first.
   */
  it('leaves the fitted content smaller than the pane it was fitted to', () => {
    const pane = { width: 900, height: 600 };
    const content = { width: 1200, height: 900 };
    const zoom = fitZoom(pane, content);
    expect(content.width * zoom).toBeLessThan(pane.width);
    expect(content.height * zoom).toBeLessThan(pane.height);
  });

  it('shrinks to fit but never magnifies', () => {
    expect(fitZoom({ width: 2000, height: 2000 }, { width: 300, height: 200 })).toBe(1);
  });

  it('answers a laid-out-nothing content with the identity', () => {
    expect(fitZoom({ width: 900, height: 600 }, { width: 0, height: 0 })).toBe(1);
  });
});

describe('anchoredScroll', () => {
  it('keeps the point under the cursor under the cursor', () => {
    // 300px into a canvas drawn at 1×, zooming to 2×: that point is now 600px
    // in, so the scroller owes the 300px difference.
    expect(anchoredScroll(0, 300, 1, 2)).toBe(300);
  });

  it('does not move the scroller for a point on the leading edge', () => {
    expect(anchoredScroll(120, 0, 1, 2)).toBe(120);
  });

  it('pulls back when zooming out', () => {
    expect(anchoredScroll(300, 300, 2, 1)).toBeLessThan(300);
  });
});

describe('zoomPercent', () => {
  it('reads as whole percent', () => {
    expect(zoomPercent(0.92)).toBe('92%');
    expect(zoomPercent(1)).toBe('100%');
  });
});
