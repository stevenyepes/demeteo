/**
 * The claim: the discovery workspace row picks its column count from a width
 * that was actually measured and is actually wide enough to seat every pane at
 * the width it will render at — including an interview the user has dragged.
 *
 * Both failure directions cost something real. Splitting into three too
 * eagerly — or on the zeros a hidden, not-yet-laid-out row reports — wedges
 * the graph or the inspector below the width it needs to be read. Never
 * widening past `'stacked'` wastes the room a real window has. So each
 * threshold is pinned on both sides here, rather than left to be eyeballed by
 * resizing a window by hand.
 */
import { describe, expect, it } from 'vitest';

import {
  COLLAPSED_RAIL_WIDTH,
  DEFAULT_INTERVIEW_WIDTH,
  GRAPH_MIN_WIDTH,
  INSPECTOR_WIDTH,
  INTERVIEW_KEYBOARD_STEP,
  INTERVIEW_MAX_WIDTH,
  INTERVIEW_MIN_WIDTH,
  interviewWidthForKey,
  pickDiscoveryLayout,
  resolveInterviewWidth,
} from './discoveryLayout';

const H = 800;
const STACK_BELOW = INTERVIEW_MIN_WIDTH + GRAPH_MIN_WIDTH;
const threeUpAt = (interview: number) => interview + INSPECTOR_WIDTH + GRAPH_MIN_WIDTH;

describe('pickDiscoveryLayout', () => {
  it('stacks before anything has been measured', () => {
    expect(pickDiscoveryLayout(null)).toBe('stacked');
  });

  it.each([
    ['a zero width', 0, H],
    ['a negative width', -10, H],
    ['a zero height, however wide', 4000, 0],
    ['a negative height, however wide', 4000, -1],
  ])('stacks on %s', (_name, width, height) => {
    expect(pickDiscoveryLayout({ width, height })).toBe('stacked');
  });

  it('stacks one pixel below the interview minimum plus the graph minimum', () => {
    expect(pickDiscoveryLayout({ width: STACK_BELOW - 1, height: H })).toBe('stacked');
  });

  it('overlays the inspector from that width, narrowing the interview to fit', () => {
    expect(pickDiscoveryLayout({ width: STACK_BELOW, height: H })).toBe('overlay-inspector');
  });

  it('goes three-up exactly where the default interview, inspector and graph all fit', () => {
    const at = threeUpAt(DEFAULT_INTERVIEW_WIDTH);
    expect(pickDiscoveryLayout({ width: at - 1, height: H })).toBe('overlay-inspector');
    expect(pickDiscoveryLayout({ width: at, height: H })).toBe('three-up');
  });

  it('moves the three-up threshold with the width the user dragged the interview to', () => {
    const wide = 700;
    expect(pickDiscoveryLayout({ width: threeUpAt(DEFAULT_INTERVIEW_WIDTH), height: H }, false, wide)).toBe(
      'overlay-inspector',
    );
    expect(pickDiscoveryLayout({ width: threeUpAt(wide), height: H }, false, wide)).toBe('three-up');
  });

  describe('with the interview hidden', () => {
    it('seats the inspector in flow beside the rail', () => {
      const at = COLLAPSED_RAIL_WIDTH + INSPECTOR_WIDTH + GRAPH_MIN_WIDTH;
      expect(pickDiscoveryLayout({ width: at - 1, height: H }, true)).toBe('overlay-inspector');
      expect(pickDiscoveryLayout({ width: at, height: H }, true)).toBe('three-up');
    });

    it('ignores the dragged interview width, which the rail does not occupy', () => {
      const at = COLLAPSED_RAIL_WIDTH + INSPECTOR_WIDTH + GRAPH_MIN_WIDTH;
      expect(pickDiscoveryLayout({ width: at, height: H }, true, INTERVIEW_MAX_WIDTH)).toBe('three-up');
    });

    it('still stacks below the graph minimum, where there is nothing to reclaim into', () => {
      expect(
        pickDiscoveryLayout({ width: COLLAPSED_RAIL_WIDTH + GRAPH_MIN_WIDTH - 1, height: H }, true),
      ).toBe('stacked');
    });

    it('reclaims nothing from an unmeasured row', () => {
      expect(pickDiscoveryLayout(null, true)).toBe('stacked');
      expect(pickDiscoveryLayout({ width: 0, height: 0 }, true)).toBe('stacked');
    });
  });
});

describe('resolveInterviewWidth', () => {
  it('keeps a width inside its bounds as asked', () => {
    expect(resolveInterviewWidth(500, 2000)).toBe(500);
  });

  it('never goes below the interview minimum or above its maximum', () => {
    expect(resolveInterviewWidth(10, 2000)).toBe(INTERVIEW_MIN_WIDTH);
    expect(resolveInterviewWidth(5000, 4000)).toBe(INTERVIEW_MAX_WIDTH);
  });

  // Widths, not modes: the graph is `flex-1 min-w-0`, so it shrinks silently
  // rather than overflowing, and nothing else reports it is too narrow to read.
  it('leaves the graph its minimum on a row narrower than the interview asked for', () => {
    const row = 900;
    expect(row - resolveInterviewWidth(800, row)).toBe(GRAPH_MIN_WIDTH);
  });

  it('applies only its own bounds to an unmeasured row', () => {
    expect(resolveInterviewWidth(600, 0)).toBe(600);
  });

  it('falls back to the default for a width that is not a number', () => {
    expect(resolveInterviewWidth(Number.NaN, 2000)).toBe(DEFAULT_INTERVIEW_WIDTH);
  });
});

describe('interviewWidthForKey', () => {
  it('grows on Right and shrinks on Left, by one step', () => {
    expect(interviewWidthForKey('ArrowRight', 500, 2000)).toBe(500 + INTERVIEW_KEYBOARD_STEP);
    expect(interviewWidthForKey('ArrowLeft', 500, 2000)).toBe(500 - INTERVIEW_KEYBOARD_STEP);
  });

  it('jumps to either bound on Home and End', () => {
    expect(interviewWidthForKey('Home', 500, 2000)).toBe(INTERVIEW_MIN_WIDTH);
    expect(interviewWidthForKey('End', 500, 2000)).toBe(INTERVIEW_MAX_WIDTH);
  });

  it('leaves every other key to the rest of the app', () => {
    expect(interviewWidthForKey('Enter', 500, 2000)).toBeNull();
  });
});
