import { useEffect, useRef } from 'react';

/**
 * Refs for each timeline step card, plus the scroll that uses them.
 *
 * The active gate card is scrolled into view once the timeline data has
 * loaded and the matching card has mounted, so the gate the user was routed
 * here to decide cannot be missed even if a stale `awaiting_gate` chip is
 * still rendered on a sibling card. Only fires when `gateStepExecutionId` is
 * set — i.e. we arrived from a `gate_required` event or a Decide Gate click.
 */
export function useGateCardScroll(gateStepExecutionId: string | null | undefined, stepCount: number) {
  const stepCardRefs = useRef<Record<string, HTMLDivElement | null>>({});

  useEffect(() => {
    if (!gateStepExecutionId) return;
    const el = stepCardRefs.current[gateStepExecutionId];
    if (el) {
      el.scrollIntoView({ behavior: 'smooth', block: 'center' });
    }
  }, [gateStepExecutionId, stepCount]);

  return stepCardRefs;
}
