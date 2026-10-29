import { Chip } from '../../../src/components/ui/Chip';
import { buttonClasses } from '../../../src/components/ui/Button';
import { elapsedText, type NeedsYouItem } from '../lib/fleetSummary';
import { routeHref } from '../lib/router';
import { ToneCard } from './ToneCard';

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
  const details = [item.projectName, item.stepTitle ?? item.stepId, `waiting ${elapsedText(item.since, now)}`].filter(
    (part) => part !== null,
  );
  return (
    <ToneCard tone="amber" surface="panel" needsYou className="flex items-center gap-4 py-4 pr-4">
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <div className="flex flex-wrap items-center gap-2">
          <Chip tone="amber" dot>
            Gate needs you
          </Chip>
          <span className="font-mono text-xs text-slate-400 break-all">{item.instanceLabel}</span>
        </div>
        {item.title === null ? (
          <span className="font-mono text-sm text-slate-200 break-all">{item.featureId}</span>
        ) : (
          <span className="font-heading text-base font-semibold text-white break-words">{item.title}</span>
        )}
        <span className="font-mono text-xs text-slate-400 break-all">{details.join(' · ')}</span>
      </div>
      <a
        href={routeHref({ name: 'run', instanceId: item.instanceId, featureId: item.featureId })}
        aria-label={`Review ${item.title ?? item.featureId} on ${item.instanceLabel}`}
        className={`shrink-0 ${buttonClasses('secondary', 'sm')}`}
      >
        Review
      </a>
    </ToneCard>
  );
}
