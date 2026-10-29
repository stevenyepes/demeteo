import { useEffect, useId, useRef, useState } from 'react';

import { buttonClasses } from '../../../src/components/ui/Button';
import { CeremonyError } from '../components/CeremonyError';
import { FleetFilters } from '../components/FleetFilters';
import { InstanceCard } from '../components/InstanceCard';
import { NeedsYouCard } from '../components/NeedsYouCard';
import { PageHeader } from '../components/PageHeader';
import { useFleet } from '../lib/fleetContext';
import {
  type FleetFilter,
  filterFleet,
  fleetFilterCounts,
  fleetSummaryText,
  liveRunsByInstance,
  needsYouOldestFirst,
  summarizeFleet,
} from '../lib/fleetSummary';
import { routeHref } from '../lib/router';
import { listRuns, type RunSummary } from '../lib/runs';

const NO_RUNS: RunSummary[] = [];

export function Fleet() {
  const { fleet, failure } = useFleet();
  const runs = useLiveRuns();
  const [filter, setFilter] = useState<FleetFilter>('all');
  const [query, setQuery] = useState('');

  const needsYouId = useId();
  const now = Math.floor(Date.now() / 1000);
  const summary = summarizeFleet(fleet ?? []);
  const needsYou = needsYouOldestFirst(fleet ?? []);
  const populated = fleet !== null && fleet.length > 0;
  const runsOf = (instanceId: string) => runs?.get(instanceId) ?? NO_RUNS;
  const shown = fleet === null ? [] : filterFleet(fleet, runsOf, filter, query);

  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        eyebrow="Personal fleet"
        title="Fleet"
        summary={
          populated ? (
            <section aria-label="Fleet summary" className="flex flex-col gap-1">
              <span>{fleetSummaryText(summary)}</span>
              {summary.withheld > 0 && (
                <span className="font-sans text-xs">
                  {summary.withheld === 1
                    ? '1 instance has read off, so its runs and gates are not counted.'
                    : `${summary.withheld} instances have read off, so their runs and gates are not counted.`}
                </span>
              )}
            </section>
          ) : undefined
        }
        actions={
          populated ? (
            <>
              <a href={routeHref({ name: 'add-instance' })} className={buttonClasses('secondary', 'lg')}>
                Add instance
              </a>
              <a href={routeHref({ name: 'dispatch' })} className={buttonClasses('primary', 'lg')}>
                Run a pipeline
              </a>
            </>
          ) : undefined
        }
      />
      {fleet === null && failure === null && <p className="text-sm text-slate-400">Loading your instances…</p>}
      {fleet === null && failure && (
        <div className="max-w-sm">
          <CeremonyError error={failure.error} />
        </div>
      )}
      {fleet !== null && fleet.length === 0 && (
        <section className="panel flex flex-col items-start gap-4 p-6 text-sm text-slate-400">
          <p>No instances are paired with this Hub yet. Pairing starts from the desktop.</p>
          <a href={routeHref({ name: 'add-instance' })} className={buttonClasses('primary')}>
            Add instance
          </a>
        </section>
      )}
      {populated && needsYou.length > 0 && (
        <section aria-labelledby={needsYouId} className="flex flex-col gap-3">
          <h2 id={needsYouId} className="font-heading text-base font-semibold text-amber-400">
            Needs you
          </h2>
          <ul className="grid grid-cols-[repeat(auto-fit,minmax(min(460px,100%),1fr))] gap-3">
            {needsYou.map((item) => (
              <li key={`${item.instanceId}/${item.stepExecutionId}`}>
                <NeedsYouCard item={item} now={now} />
              </li>
            ))}
          </ul>
        </section>
      )}
      {populated && (
        <section aria-label="Instances" className="flex flex-col gap-4">
          <FleetFilters
            filter={filter}
            counts={fleetFilterCounts(fleet)}
            query={query}
            onFilter={setFilter}
            onQuery={setQuery}
          />
          {shown.length === 0 ? (
            <p className="text-sm text-slate-400">No instance matches.</p>
          ) : (
            <ul className="grid grid-cols-[repeat(auto-fill,minmax(min(380px,100%),1fr))] gap-4">
              {shown.map((instance) => (
                <li key={instance.id}>
                  <InstanceCard instance={instance} runs={runs === null ? null : runsOf(instance.id)} now={now} />
                </li>
              ))}
            </ul>
          )}
        </section>
      )}
    </div>
  );
}

/**
 * The fleet carries a count of live runs, not the runs, so the rows on each
 * card come from the run list. A failed read is not shown as an error: the
 * cards fall back to the fleet's count, which is still true.
 */
function useLiveRuns(): Map<string, RunSummary[]> | null {
  const [runs, setRuns] = useState<Map<string, RunSummary[]> | null>(null);

  /** Bumped on unmount, so a read that settles after it has no page to land on. */
  const latestRead = useRef(0);

  useEffect(() => {
    latestRead.current += 1;
    const read = latestRead.current;
    listRuns({ status: 'running' }).then(
      (answer) => {
        if (latestRead.current === read) setRuns(liveRunsByInstance(answer));
      },
      () => {},
    );
    return () => {
      latestRead.current += 1;
    };
  }, []);

  return runs;
}
