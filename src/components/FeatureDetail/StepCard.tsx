import { memo, useCallback } from 'react';
import {
  ArrowRight, CheckCircle, Cpu, Hourglass, RefreshCw, ShieldAlert, XCircle,
} from 'lucide-react';
import type { StepExecution } from '../../types';
import type { DensityClasses } from '../../lib/density';
import type { EffortLevel } from '../../lib/effortLevels';
import { TONE_CHIP } from '../../lib/runStatus';
import { AssignmentChips } from '../ui/AssignmentChips';
import { StepMetrics } from './StepMetrics';
import { humanizeStepId } from './stepIdentity';

interface StepCardProps {
  step: StepExecution;
  index: number;
  isActiveGate: boolean;
  /** True while this step is the one the inspector is reading. */
  isSelected: boolean;
  cardRef: (el: HTMLDivElement | null) => void;
  density: DensityClasses;
  agentKind?: string | null;
  effort?: EffortLevel | null;
  onSelect: (stepExecutionId: string) => void;
  onDecideGate: (stepExecutionId: string) => void;
}

function StepCardInner({
  step,
  index,
  isActiveGate,
  isSelected,
  cardRef,
  density,
  agentKind,
  effort,
  onSelect,
  onDecideGate,
}: StepCardProps) {
  let icon = <Hourglass className="w-4 h-4 text-slate-500" />;
  let statusBg = 'border-white/5 bg-white/[0.01]';

  if (step.status === 'completed') {
    icon = <CheckCircle className="w-4 h-4 text-emerald-400" />;
    statusBg = 'border-emerald-500/20 bg-emerald-950/5';
  } else if (step.status === 'failed') {
    icon = <XCircle className="w-4 h-4 text-rose-400" />;
    statusBg = 'border-rose-500/20 bg-rose-950/5';
  } else if (step.status === 'running') {
    icon = <Cpu className="w-4 h-4 text-cyan-400 animate-spin" />;
    statusBg = 'border-cyan-500/30 bg-cyan-950/10 shadow-[0_0_15px_rgba(6,182,212,0.05)]';
  } else if (step.status === 'verifying') {
    icon = <RefreshCw className="w-4 h-4 text-violet-400 animate-spin" />;
    statusBg = 'border-violet-500/30 bg-violet-950/10 shadow-[0_0_15px_rgba(139,92,246,0.05)]';
  } else if (step.status === 'awaiting_gate') {
    icon = <ShieldAlert className="w-4 h-4 text-amber-400" />;
    statusBg = 'border-amber-500/40 bg-amber-950/10 shadow-[0_0_15px_rgba(245,158,11,0.08)]';
  }

  const selectSelf = useCallback(() => onSelect(step.id), [onSelect, step.id]);
  const decideGate = useCallback(() => onDecideGate(step.id), [onDecideGate, step.id]);
  const stepName = humanizeStepId(step.step_id);

  return (
    <li className="relative group">
      {/* Connector node circle */}
      <span className="absolute -left-[41px] top-1.5 flex items-center justify-center w-6 h-6 rounded-full bg-[#08090c] border border-white/10">
        <span className="text-[10px] text-slate-400 font-bold">{index + 1}</span>
      </span>

      {/* `timeline-step-card` may not move up to the `li`: `content-visibility`
          implies paint containment on screen as well as off it, and the circle
          above hangs outside this box — contained on the `li`, the timeline's
          spine is clipped away. */}
      <div
        ref={cardRef}
        data-step-id={step.id}
        className={`timeline-step-card rounded-xl border transition-all duration-300 ${density.card} ${statusBg} ${ring(isActiveGate, isSelected)}`}
      >
        {/* Title and metrics are one row while they fit and two
            when they don't — a half-width window squeezes the
            metric group until the last value renders outside the
            card. (It used to be the artifact split column that
            did this; the window alone is reason enough.) */}
        <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
          {/* Wraps rather than truncates: with a truncate here it spent the
              row's spare width shortening the step name to "Resea…" in a card
              that had room for it. */}
          <div className="flex min-w-0 flex-wrap items-center gap-x-1 gap-y-1">
            {/* The row's identity *is* the select control, so the affordance
                sits on the thing the user reads rather than on a separate
                widget beside it. The gate CTA below stays outside it — nesting
                it would make one button contain another. */}
            <button
              type="button"
              data-step-row={step.id}
              aria-current={isSelected ? 'step' : undefined}
              onClick={selectSelf}
              className={`-mx-2 flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border px-2 py-1 text-left transition ${
                isSelected ? TONE_CHIP.cyan : 'border-transparent hover:bg-white/5'
              }`}
            >
              <span className="shrink-0">{icon}</span>
              <span className={`font-semibold text-white tracking-wide break-words ${density.title}`}>
                {stepName}
              </span>
              <span className="text-[9px] px-2 py-0.5 rounded bg-white/5 text-slate-400 font-mono shrink-0">
                {step.step_kind}
              </span>
              <AssignmentChips subject={stepName} agentKind={agentKind} effort={effort} />
              {(step.iteration_count ?? 0) > 0 && (
                <span
                  className="flex items-center gap-1 text-[9px] px-2 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20 font-mono"
                  title={`This step has been retried ${step.iteration_count} time${step.iteration_count !== 1 ? 's' : ''}`}
                >
                  <RefreshCw className="w-2.5 h-2.5" />
                  {step.iteration_count}x
                </span>
              )}
            </button>
          </div>

          <StepMetrics step={step} sizeClass={density.metrics} />
        </div>

        {/* The row's one CTA, and the only detail left on the card
            (UI_REDESIGN_PLAN §6, Phase 3). A waiting gate is the run asking the
            user a question, so it may not wait behind a selection the way a
            failure's output or a running step's stream now do. Everything else
            — stream, artifacts, retry, environment remediation — is the
            inspector's, which is the fix rather than a relocation: an expanding
            card reflows every sibling below it and can still only show one step
            at a time (§3.1). */}
        {step.status === 'awaiting_gate' && (
          <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
            <span className="min-w-0 flex-1 text-xs text-amber-400 font-semibold uppercase tracking-wide">
              Pipeline paused. Awaiting manual review.
            </span>
            <button
              onClick={decideGate}
              className="flex shrink-0 items-center gap-1.5 whitespace-nowrap px-3 py-1.5 bg-amber-500 hover:bg-amber-600 rounded text-xs font-bold text-black transition"
            >
              Decide Gate <ArrowRight className="w-3 h-3" />
            </button>
          </div>
        )}
      </div>
    </li>
  );
}

/** An awaiting gate outranks the selection: the gate ring is the run telling
 *  the user it is stuck, and the selection is only where they happen to be
 *  looking. Two rings on one card read as one indeterminate colour. */
function ring(isActiveGate: boolean, isSelected: boolean): string {
  if (isActiveGate) return 'ring-2 ring-amber-500/40';
  return isSelected ? 'ring-1 ring-cyan-500/40' : '';
}

/**
 * Memoized, and every prop above is a primitive or a stable identity for that
 * reason: a run re-renders its timeline on every reload tick and on every
 * selection change, and the only cards whose props changed are the ones that
 * moved. A literal object, array or closure in any prop here re-opens the
 * fan-out (UI_REDESIGN_PLAN §4.2, §4.6) — `density` is safe only because
 * `densityClasses` returns one stable object per density.
 */
export const StepCard = memo(StepCardInner);
