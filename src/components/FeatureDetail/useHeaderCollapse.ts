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
 *
 * **`gateStepExecutionId` is an input because one scroll here is not a
 * reading gesture.** Routing to a gate runs `useGateCardScroll`, which centres
 * the gate card underneath the full-window overlay — so the column arrives
 * scrolled deep into the run without the user having touched it, and closing
 * the gate hands back a header with the feature id already gone. Nothing
 * re-fires on close (the scroll is over, and the offset that would restore the
 * id is one the user has no reason to suspect they need), so the id stays
 * missing until the whole view remounts. Re-establishing the header whenever
 * the overlay opens or closes trades a transient disagreement with the offset —
 * settled by the next real scroll — for a header that always names what is
 * under it.
 */
export function useHeaderCollapse(
  runColumnEl: HTMLElement | null,
  gateStepExecutionId?: string | null,
): boolean {
  const [collapsed, setCollapsed] = useState(false);

  useEffect(() => {
    setCollapsed(false);
  }, [gateStepExecutionId]);

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
