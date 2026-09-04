import { useEffect, useState } from 'react';

import { type DiscoveryLayoutMode, type DiscoveryRowSize, pickDiscoveryLayout } from './discoveryLayout';

/**
 * Measures the discovery workspace row and hands `pickDiscoveryLayout` the
 * size it decides from. Same discipline as `useHeaderDensity`: the callback
 * reads `el.offsetWidth` / `el.offsetHeight` rather than `entry.contentRect`,
 * because jsdom's `ResizeObserverStub` fires with an empty entry list — that
 * is the only reason the three-band ladder is reachable from a test at all.
 *
 * The element is held in state rather than a ref, so the render that produced
 * the node is what arms the observer; a ref write triggers no effect. Both
 * dimensions are rounded to the nearest 8px and the state update is guarded
 * by a `prev`-returning identity check, so a resize drag doesn't cascade a
 * re-render per frame (`useRunColumnLayout.ts`'s pattern).
 */
export function useDiscoveryColumnLayout(): {
  setRowEl: (el: HTMLDivElement | null) => void;
  rowSize: DiscoveryRowSize | null;
  layoutMode: DiscoveryLayoutMode;
} {
  const [rowEl, setRowEl] = useState<HTMLDivElement | null>(null);
  const [rowSize, setRowSize] = useState<DiscoveryRowSize | null>(null);

  useEffect(() => {
    if (!rowEl || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      const next = {
        width: Math.round(rowEl.offsetWidth / 8) * 8,
        height: Math.round(rowEl.offsetHeight / 8) * 8,
      };
      setRowSize((prev) => (prev && prev.width === next.width && prev.height === next.height ? prev : next));
    });
    observer.observe(rowEl);
    return () => observer.disconnect();
  }, [rowEl]);

  return { setRowEl, rowSize, layoutMode: pickDiscoveryLayout(rowSize) };
}
