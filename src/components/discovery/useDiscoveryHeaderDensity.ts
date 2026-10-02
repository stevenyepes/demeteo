import { useEffect, useState } from 'react';

import { type DiscoveryHeaderDensity, nextDiscoveryHeaderDensity } from '../../lib/discoveryHeaderLayout';

/**
 * Measures the Discovery workspace header and hands
 * `src/lib/discoveryHeaderLayout.ts` the width it decides from — the same
 * discipline as `src/hooks/useHeaderDensity.ts`, whose doc carries why the
 * element is held in state and why the callback reads `offsetWidth`.
 *
 * Starts `wide`, which is what jsdom keeps (it reports a 0 width), so a test
 * that does not drive the observer sees every action inline.
 */
export function useDiscoveryHeaderDensity(): {
  setHeaderEl: (el: HTMLElement | null) => void;
  density: DiscoveryHeaderDensity;
} {
  const [headerEl, setHeaderEl] = useState<HTMLElement | null>(null);
  const [density, setDensity] = useState<DiscoveryHeaderDensity>('wide');

  useEffect(() => {
    if (!headerEl || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      const width = headerEl.offsetWidth;
      setDensity((prev) => nextDiscoveryHeaderDensity(width, prev));
    });
    observer.observe(headerEl);
    return () => observer.disconnect();
  }, [headerEl]);

  return { setHeaderEl, density };
}
