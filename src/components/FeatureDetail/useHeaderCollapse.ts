import { useEffect, useState } from 'react';

import { nextHeaderCollapsed } from '../../lib/headerCollapse';

/**
 * The scroll offset the collapsing feature header reacts to
 * (UI_REDESIGN_PLAN §6, Phase 3 — deferred out of Phase 2).
 *
 * **Which element scrolls depends on the inspector's layout.** Stacked, the run
 * column carries the scroll; side by side, `RunPanes` gives the run surface its
 * own `overflow-y-auto` box inside a fixed-height row and the column stops
 * scrolling at all. A listener bound to the column alone therefore collapses the
 * header in one layout and never fires in the other — and the layout flips on a
 * window resize, so it would read as intermittent rather than as missing.
 *
 * Scroll events do not bubble, which is what makes the capture phase the fix
 * rather than a trick: one listener on the column sees a descendant's scroll on
 * the way down. `data-run-scroll` is what narrows that to the run surface —
 * without it the inspector's own tab bodies and the meta track would drive the
 * header too, and a user reading an artifact would watch the title shrink.
 */
export function useHeaderCollapse(runColumnEl: HTMLElement | null): boolean {
  const [collapsed, setCollapsed] = useState(false);

  useEffect(() => {
    if (!runColumnEl) return;
    let frame: number | null = null;

    const onScroll = (event: Event) => {
      const scroller = event.target;
      if (!(scroller instanceof HTMLElement) || !scroller.hasAttribute('data-run-scroll')) return;
      if (frame !== null) return;
      frame = requestAnimationFrame(() => {
        frame = null;
        setCollapsed((was) => nextHeaderCollapsed(scroller.scrollTop, was));
      });
    };

    runColumnEl.addEventListener('scroll', onScroll, true);
    return () => {
      runColumnEl.removeEventListener('scroll', onScroll, true);
      if (frame !== null) cancelAnimationFrame(frame);
    };
  }, [runColumnEl]);

  return collapsed;
}
