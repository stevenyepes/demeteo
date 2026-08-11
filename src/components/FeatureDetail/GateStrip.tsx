import { useMemo } from 'react';
import { ArrowRight, ShieldAlert } from 'lucide-react';

import { awaitingGates, type GateStripRow } from '../../lib/gateStrip';
import { TONE_CHIP } from '../../lib/runStatus';
import { humanizeStepId } from './stepIdentity';

export interface GateStripProps {
  steps: readonly GateStripRow[];
  onDecideGate: (stepExecutionId: string) => void;
  className?: string;
}

/**
 * The run chrome's standing answer to "is anything waiting on me?"
 * (`docs/UI_REDESIGN_PLAN.md` §3.2).
 *
 * A gate is the one thing in the app that cannot progress without the user, and
 * until this existed the only affordance was a block inside whichever step card
 * happened to be waiting — findable by scrolling. So the strip lives in the
 * chrome, above the scroll, and the card keeps none of it.
 *
 * It acts on the *earliest* open gate rather than offering one CTA per gate: a
 * strip that grows a row per branch stops being chrome, and gates are decided
 * in run order anyway. The count is what says the other ones exist.
 *
 * Only the dot animates. `src/App.css` records a WKWebView GPU incident behind
 * that rule and the whole-block `animate-pulse` this replaces was an instance
 * of it.
 */
export function GateStrip({
  steps,
  onDecideGate,
  className = '',
}: GateStripProps): React.ReactElement | null {
  const gates = useMemo(() => awaitingGates(steps), [steps]);

  const next = gates[0];
  if (!next) return null;

  const stepName = humanizeStepId(next.step_id);

  return (
    <div
      data-testid="gate-strip"
      role="status"
      // The CTA sits next to the message, not at the far end of the strip. The
      // strip spans the window, and pinning the button right put a metre of
      // empty amber between "1 gate needs you" and the thing that answers it on
      // a 4K display — a banner reads as one sentence or it reads as two
      // unrelated things.
      className={`flex flex-wrap items-center gap-x-4 gap-y-2 rounded-xl border px-4 py-2.5 ${TONE_CHIP.amber} ${className}`}
    >
      <div className="flex min-w-0 items-center gap-2.5">
        <span
          aria-hidden="true"
          className="h-2 w-2 shrink-0 rounded-full bg-current animate-pulse-glow-amber"
        />
        <ShieldAlert className="w-4 h-4 shrink-0" />
        <span className="text-xs font-semibold uppercase tracking-wide">
          {gates.length === 1 ? '1 gate needs you' : `${gates.length} gates need you`}
        </span>
        <span className="truncate font-mono text-xs opacity-80" title={stepName}>
          {stepName}
        </span>
      </div>

      <button
        type="button"
        onClick={() => onDecideGate(next.id)}
        className="flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded bg-amber-500 px-3 py-1.5 text-xs font-bold text-black transition hover:bg-amber-600"
      >
        Decide Gate <ArrowRight className="w-3 h-3" />
      </button>
    </div>
  );
}
