import type { StepExecution } from '../../types';
import { formatCost, formatDuration, formatTokens } from '../../lib/utils';

/**
 * The scan tier's right-hand group: what the step spent and how long it took
 * (UI_REDESIGN_PLAN §3.3's first tier, applied to a run row).
 *
 * Its own component, and rendered by `StepCard` unconditionally — a pending step
 * with no numbers yet still mounts an empty one. That is what the timeline's
 * render-count test counts, and a counter hanging off a child the card renders
 * only *sometimes* stops covering the cards that lack it without failing.
 */
export function StepMetrics({ step, sizeClass }: { step: StepExecution; sizeClass: string }) {
  return (
    <div
      data-step-metrics={step.id}
      className={`flex shrink-0 flex-wrap items-center justify-end gap-x-4 gap-y-1 font-mono ${sizeClass}`}
    >
      {typeof step.cost_usd === 'number' && step.cost_usd > 0 && (
        <span className="text-emerald-400 whitespace-nowrap" title={`${step.cost_usd.toFixed(4)} USD`}>
          {formatCost(step.cost_usd)}
        </span>
      )}
      {typeof step.cache_read_input_tokens === 'number' && step.cache_read_input_tokens > 0 && (
        <span
          className="text-violet-400 whitespace-nowrap"
          title={`${step.cache_read_input_tokens.toLocaleString()} cache-read tokens (live from last turn)`}
        >
          {formatTokens(step.cache_read_input_tokens)}p cache
        </span>
      )}
      {typeof step.tokens === 'number' && (
        <span className="text-cyan-400 whitespace-nowrap">{formatTokens(step.tokens)}</span>
      )}
      {typeof step.wall_clock_secs === 'number' && (
        <span className="text-slate-400 whitespace-nowrap">{formatDuration(step.wall_clock_secs)}</span>
      )}
    </div>
  );
}
