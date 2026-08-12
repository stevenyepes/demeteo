import { useEffect, useState } from 'react';

import { type HeaderDensity, nextHeaderDensity } from '../lib/headerLayout';

/**
 * Measures the top header bar and hands `src/lib/headerLayout.ts` the width it
 * decides from. Which element to observe is the whole decision, and it is
 * documented there — not here.
 *
 * The element is held in state rather than in a ref, so the render that
 * produced the node is what arms the observer; a ref write triggers nothing and
 * the effect would run once against `null`. The callback reads `el.offsetWidth`
 * instead of `entry.contentRect`, which is what lets a tick carrying no entries
 * still answer — jsdom's observer double fires exactly that way, and it is the
 * only reason the ladder is reachable from a test at all.
 *
 * `nextHeaderDensity` returns its `prev` by identity inside the band, so React
 * bails out and a resize drag renders nothing below a component that sits above
 * every view (`docs/UI_REDESIGN_PLAN.md` §4.1).
 */
export function useHeaderDensity(): {
  setHeaderEl: (el: HTMLElement | null) => void;
  density: HeaderDensity;
} {
  const [headerEl, setHeaderEl] = useState<HTMLElement | null>(null);
  const [density, setDensity] = useState<HeaderDensity>('labels');

  useEffect(() => {
    if (!headerEl || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      const width = headerEl.offsetWidth;
      setDensity((prev) => nextHeaderDensity(width, prev));
    });
    observer.observe(headerEl);
    return () => observer.disconnect();
  }, [headerEl]);

  return { setHeaderEl, density };
}
