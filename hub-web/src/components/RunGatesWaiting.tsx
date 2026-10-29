import { useId } from 'react';

import { Button } from '../../../src/components/ui/Button';
import { Chip } from '../../../src/components/ui/Chip';
import { elapsedText } from '../lib/fleetSummary';
import type { RunDetail } from '../lib/runs';
import { ToneCard } from './ToneCard';

interface RunGatesWaitingProps {
  detail: Pick<RunDetail, 'pendingGates'>;
  titles: Readonly<Record<string, string>>;
  /** Unix seconds, injected so the waiting time is testable. */
  now: number;
  onSelect: (stepId: string) => void;
}

/**
 * The inspector's place while no step is chosen. A gate naming no step has no
 * node to open, so it is listed without a Review action: the decision needs
 * the step's inspector.
 */
export function RunGatesWaiting({ detail, titles, now, onSelect }: RunGatesWaitingProps) {
  const headingId = useId();
  const gates = detail.pendingGates;
  return (
    <section aria-labelledby={headingId} className="panel flex flex-col gap-3 p-5 text-sm text-slate-400">
      <h2 id={headingId} className="font-heading text-base font-semibold text-white">
        {gates.length === 0 ? 'Inspector' : 'Waiting on you'}
      </h2>
      {gates.length === 0 ? (
        <p>Choose a step to see its state, change who runs it, or decide its gate.</p>
      ) : (
        <ul aria-label="Gates waiting" className="flex flex-col gap-2">
          {gates.map((gate) => {
            const { stepId } = gate;
            const name = stepId === null ? null : (titles[stepId] ?? stepId);
            return (
              <li key={gate.stepExecutionId}>
                <ToneCard tone="amber" surface="nested" needsYou className="flex items-center gap-3 py-3 pr-3">
                  <span className="flex min-w-0 flex-1 flex-col gap-1">
                    <Chip status="awaiting_gate" className="self-start">
                      Gate needs you
                    </Chip>
                    <span className="text-slate-200 break-words">{name ?? 'A gate on no reported step'}</span>
                    <span className="font-mono text-xs">waiting {elapsedText(gate.since, now)}</span>
                  </span>
                  {stepId !== null && (
                    <Button size="sm" aria-label={`Review ${name}`} onClick={() => onSelect(stepId)}>
                      Review
                    </Button>
                  )}
                </ToneCard>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
