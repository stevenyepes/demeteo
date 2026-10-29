import { useState } from 'react';

import type { GateDecisionTicket } from '../components/StepInspector';

/**
 * Held by the run, not by whichever panel sent the decision: the mirror drops
 * a gate once the instance acts on it, which is exactly when `accepted` arrives.
 */
export function useGateDecisions() {
  const [gateDecisions, setGateDecisions] = useState<Record<string, GateDecisionTicket>>({});
  const onGateDecided = (stepExecutionId: string, decided: GateDecisionTicket) => {
    // Re-inserted, so the step's latest decision is its last entry.
    setGateDecisions(({ [stepExecutionId]: _replaced, ...rest }) => ({ ...rest, [stepExecutionId]: decided }));
  };
  const onGateSettled = (requestId: string) => {
    setGateDecisions((held) => {
      const entry = Object.entries(held).find(([, decided]) => decided.ticket.requestId === requestId);
      if (entry === undefined || entry[1].settled) return held;
      return { ...held, [entry[0]]: { ...entry[1], settled: true } };
    });
  };
  return { gateDecisions, onGateDecided, onGateSettled };
}
