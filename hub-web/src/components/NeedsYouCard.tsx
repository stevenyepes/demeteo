import { elapsedText, type NeedsYouItem } from '../lib/fleetSummary';
import { routeHref } from '../lib/router';

interface NeedsYouCardProps {
  item: NeedsYouItem;
  /** Unix seconds, injected so the waiting time is testable. */
  now: number;
}

/**
 * A pointer to the gate, never a decision on it: approving happens on the Run
 * route, where the step's output can be read first.
 */
export function NeedsYouCard({ item, now }: NeedsYouCardProps) {
  return (
    <article className="nested-card flex flex-wrap items-center gap-3 border-amber-500/30 bg-amber-500/10 px-4 py-3">
      <span className="flex items-center gap-2 rounded-full border border-amber-500/30 bg-amber-500/10 px-2 py-0.5 text-xs font-medium text-amber-500">
        <span aria-hidden="true" className="h-2 w-2 shrink-0 rounded-full bg-amber-500 animate-pulse-glow-amber" />
        Gate needs you
      </span>
      <span className="font-medium text-slate-200 break-all">{item.instanceLabel}</span>
      <span className="font-mono text-xs text-slate-300 break-all">{item.featureId}</span>
      {item.stepId !== null && <span className="font-mono text-xs text-slate-400 break-all">{item.stepId}</span>}
      <span className="text-xs text-slate-400">waiting {elapsedText(item.since, now)}</span>
      <a
        href={routeHref({ name: 'run', instanceId: item.instanceId, featureId: item.featureId })}
        aria-label={`Review ${item.featureId} on ${item.instanceLabel}`}
        className="ml-auto rounded-md border border-violet-500/40 bg-violet-500/10 px-3 py-1.5 text-xs font-medium text-violet-200 transition-colors hover:border-cyan-500/40 hover:text-cyan-200 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400"
      >
        Review
      </a>
    </article>
  );
}
