import { useEffect, useId, useRef, useState } from 'react';

import { CeremonyError } from '../components/CeremonyError';
import { InstanceCard } from '../components/InstanceCard';
import { NeedsYouCard } from '../components/NeedsYouCard';
import { PlaceholderCard } from '../components/PlaceholderCard';
import { type FleetInstance, getFleet } from '../lib/fleet';
import { needsYouOldestFirst, summarizeFleet } from '../lib/fleetSummary';
import { routeHref } from '../lib/router';

export function Fleet() {
  const [fleet, setFleet] = useState<FleetInstance[] | null>(null);
  const [failure, setFailure] = useState<{ error: unknown } | null>(null);

  /** Bumped on unmount, so a read that settles after it has no page to land on. */
  const latestRead = useRef(0);

  useEffect(() => {
    latestRead.current += 1;
    const read = latestRead.current;
    getFleet().then(
      (instances) => {
        if (latestRead.current === read) setFleet(instances);
      },
      (error: unknown) => {
        if (latestRead.current === read) setFailure({ error });
      },
    );
    return () => {
      latestRead.current += 1;
    };
  }, []);

  const needsYouId = useId();
  const now = Math.floor(Date.now() / 1000);
  const summary = summarizeFleet(fleet ?? []);
  const needsYou = needsYouOldestFirst(fleet ?? []);
  const stats = [
    { term: 'Instances', value: summary.instances },
    { term: 'Online', value: summary.online },
    { term: 'Runs active', value: summary.runsActive },
    { term: 'Need you', value: summary.gatesWaiting },
  ];

  return (
    <PlaceholderCard title="Fleet">
      {fleet === null && failure === null && <p>Loading your instances…</p>}
      {failure && (
        <div className="max-w-sm">
          <CeremonyError error={failure.error} />
        </div>
      )}
      {fleet !== null && fleet.length === 0 && (
        <>
          <p>No instances are paired with this Hub yet. Pairing starts from the desktop.</p>
          <a
            href={routeHref({ name: 'add-instance' })}
            className="btn-primary mt-4 inline-block focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400"
          >
            Add instance
          </a>
        </>
      )}
      {fleet !== null && fleet.length > 0 && (
        <>
          <section aria-label="Fleet summary" className="flex flex-col gap-2">
            <dl className="flex flex-wrap gap-x-8 gap-y-3">
              {stats.map(({ term, value }) => (
                <div key={term} className="flex flex-col">
                  <dt className="text-xs text-slate-400">{term}</dt>
                  <dd className="font-heading text-2xl font-bold text-white">{value}</dd>
                </div>
              ))}
            </dl>
            {summary.withheld > 0 && (
              <p className="text-xs text-slate-400">
                {summary.withheld === 1
                  ? '1 instance has read off, so its runs and gates are not counted.'
                  : `${summary.withheld} instances have read off, so their runs and gates are not counted.`}
              </p>
            )}
          </section>
          <div className="mt-4 flex flex-wrap gap-3">
            <a
              href={routeHref({ name: 'add-instance' })}
              className="btn-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400"
            >
              Add instance
            </a>
            <a
              href={routeHref({ name: 'dispatch' })}
              className="btn-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400"
            >
              Run a pipeline
            </a>
          </div>
          {needsYou.length > 0 && (
            <section aria-labelledby={needsYouId} className="mt-6">
              <h2 id={needsYouId} className="font-heading text-lg font-semibold text-amber-500">
                Needs you
              </h2>
              <ul className="mt-3 flex flex-col gap-2">
                {needsYou.map((item) => (
                  <li key={`${item.instanceId}/${item.stepExecutionId}`}>
                    <NeedsYouCard item={item} now={now} />
                  </li>
                ))}
              </ul>
            </section>
          )}
          <ul aria-label="Instances" className="mt-6 flex flex-col gap-2">
            {fleet.map((instance) => (
              <li key={instance.id}>
                <InstanceCard instance={instance} now={now} />
              </li>
            ))}
          </ul>
        </>
      )}
    </PlaceholderCard>
  );
}
