import { useRef, type MutableRefObject } from 'react';
import { RefreshCw } from 'lucide-react';
import type { RemoteRunMirror, StepExecution } from '../../types';
import { densityClasses, type Density } from '../../lib/density';
import type { RunEventAssignments } from '../../lib/runEventAssignments';
import { StepCard } from './StepCard';

const EMPTY_ASSIGNMENTS: RunEventAssignments = {};

interface StepTimelineProps {
  steps: StepExecution[];
  assignments?: RunEventAssignments;
  remoteRun: RemoteRunMirror | null;
  remoteMachineName: string | null;
  hasBootstrapPhases: boolean;
  gateStepExecutionId: string | null | undefined;
  stepCardRefs: MutableRefObject<Record<string, HTMLDivElement | null>>;
  /** Execution id the inspector *resolved* to, not the raw selection: a node id
   *  picked on the canvas matches several rows once a step has been retried,
   *  and only the attempt on screen may read as selected. */
  selectedStepId: string | null;
  density: Density;
  onSelect: (stepExecutionId: string) => void;
  onDecideGate: (stepExecutionId: string) => void;
}

/**
 * The run as a vertical list — best for skimming a long linear run, and the
 * only surface a feature with no workflow graph has.
 *
 * Clicking a row selects the step the shared inspector reads
 * (UI_REDESIGN_PLAN §3.1). The `steps.map` gets its own element so the
 * pre-hydration banner above it is not an item in the list.
 *
 * Selection is `aria-current`, deliberately not the `listbox`/`option` pair it
 * was first written as: a card carries its own gate button, and an `option` may
 * not own interactive children — so no placement of that role is valid here,
 * including on the card root.
 *
 * **Nothing here subscribes to the agent stream any more, and that is the
 * point.** Phase 1 could only *guard* the fan-out that a live stream caused
 * across every card, because the card was the thing rendering the stream; with
 * the stream mounted once in the inspector's Live tab there is no subscription
 * on this path to guard (§4.2's last bullet). A `useStreamText` call
 * reintroduced anywhere at or above this component re-opens it, memoized cards
 * or not.
 */
export function StepTimeline({
  steps,
  assignments = EMPTY_ASSIGNMENTS,
  remoteRun,
  remoteMachineName,
  hasBootstrapPhases,
  gateStepExecutionId,
  stepCardRefs,
  selectedStepId,
  density,
  onSelect,
  onDecideGate,
}: StepTimelineProps) {
  const classes = densityClasses(density);

  /** One ref callback per step, cached: a fresh closure per render would change
   *  a memoized card's props on every frame. */
  const cardRefCache = useRef(new Map<string, (el: HTMLDivElement | null) => void>());
  const cardRefFor = (stepExecutionId: string) => {
    const cached = cardRefCache.current.get(stepExecutionId);
    if (cached) return cached;
    const collect = (el: HTMLDivElement | null) => {
      stepCardRefs.current[stepExecutionId] = el;
    };
    cardRefCache.current.set(stepExecutionId, collect);
    return collect;
  };

  return (
    <div className={`relative shrink-0 border-l border-white/5 ml-4 pl-8 ${classes.list}`}>
      {remoteRun && steps.length === 0 && !hasBootstrapPhases && (
        /* Eager shadow, pre-hydration: the run was submitted a
           moment ago and the runner hasn't bootstrapped a
           feature yet, so there are no shadow steps to mirror.
           `useRemoteRun`'s poll fills this in once the runner
           reports them. Suppressed once the richer bootstrap
           stepper has phases to show. */
        <div className="glass-panel p-6 border border-cyan-500/20">
          <div className="flex items-center gap-3">
            <RefreshCw className="w-5 h-5 text-cyan-400 animate-spin shrink-0" />
            <div>
              <h3 className="text-sm font-semibold text-white">
                Submitted to {remoteMachineName ?? remoteRun.machine_id}
              </h3>
              <p className="text-xs text-slate-400 mt-1">
                The runner is cloning the repository and bootstrapping the workflow.
                Steps appear here automatically — you can close Demeteo; the run continues.
              </p>
            </div>
          </div>
        </div>
      )}
      <ul aria-label="Run steps" className={classes.list}>
        {steps.map((step, idx) => {
          const assignment = assignments[step.id];
          return (
            <StepCard
              key={step.id}
              step={step}
              index={idx}
              isActiveGate={gateStepExecutionId === step.id}
              isSelected={selectedStepId === step.id}
              cardRef={cardRefFor(step.id)}
              density={classes}
              agentKind={assignment?.agentKind}
              effort={assignment?.effort}
              onSelect={onSelect}
              onDecideGate={onDecideGate}
            />
          );
        })}
      </ul>
    </div>
  );
}
