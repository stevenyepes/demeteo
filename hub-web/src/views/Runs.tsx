import { useEffect, useId, useRef, useState } from 'react';

import { SegmentedControl } from '../../../src/components/ui/SegmentedControl';
import { CeremonyError } from '../components/CeremonyError';
import { PageHeader } from '../components/PageHeader';
import { RunPreview } from '../components/RunPreview';
import { RunRow } from '../components/RunRow';
import { useFleet } from '../lib/fleetContext';
import { listRuns, type RunSummary } from '../lib/runs';
import { bandRuns, type ListedRun, type RunsBand, type RunsBands, type RunsShown } from '../lib/runsList';

type Load = { state: 'loading' } | { state: 'loaded'; runs: RunSummary[] } | { state: 'failed'; error: unknown };

const BANDS: { band: RunsBand; heading: string; empty: string }[] = [
  { band: 'needs-you', heading: 'Needs you', empty: 'No run is waiting on you.' },
  { band: 'active', heading: 'Active', empty: 'No run is active.' },
  { band: 'finished', heading: 'Finished', empty: 'No run has finished.' },
];

interface RunsProps {
  /** The run whose preview is open; without one, the first run listed. */
  instanceId?: string;
  featureId?: string;
}

export function Runs({ instanceId, featureId }: RunsProps) {
  const [load, setLoad] = useState<Load>({ state: 'loading' });
  const [shown, setShown] = useState<RunsShown>('all');
  const { fleet } = useFleet();

  /** Bumped on unmount, so a read that settles after it has no page to land on. */
  const latestRead = useRef(0);

  useEffect(() => {
    latestRead.current += 1;
    const read = latestRead.current;
    listRuns().then(
      (runs) => {
        if (latestRead.current === read) setLoad({ state: 'loaded', runs });
      },
      (error: unknown) => {
        if (latestRead.current === read) setLoad({ state: 'failed', error });
      },
    );
    return () => {
      latestRead.current += 1;
    };
  }, []);

  const bands = load.state === 'loaded' ? bandRuns(load.runs, fleet) : null;

  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        title="Runs"
        summary={bands === null ? 'Every Feature on every instance, newest first' : summaryText(bands)}
        actions={
          bands !== null &&
          load.state === 'loaded' &&
          load.runs.length > 0 && (
            <SegmentedControl
              ariaLabel="Show"
              value={shown}
              onChange={setShown}
              options={[
                { value: 'all', label: 'All', count: load.runs.length },
                { value: 'needs-you', label: 'Needs you', count: bands['needs-you'].length, countTone: 'amber' },
                { value: 'active', label: 'Active', count: bands.active.length },
                { value: 'finished', label: 'Finished', count: bands.finished.length },
              ]}
            />
          )
        }
      />
      {load.state === 'loading' && <p className="text-sm text-slate-400">Loading runs across your fleet…</p>}
      {load.state === 'failed' && (
        <div className="max-w-sm">
          <CeremonyError error={load.error} />
        </div>
      )}
      {load.state === 'loaded' && load.runs.length === 0 && (
        <p className="text-sm text-slate-400">No runs reported by your instances yet.</p>
      )}
      {bands !== null && load.state === 'loaded' && load.runs.length > 0 && (
        <RunsLoaded bands={bands} shown={shown} instanceId={instanceId} featureId={featureId} />
      )}
      {load.state === 'loaded' && (
        <p className="text-xs text-slate-400">Runs of instances with read off are not listed.</p>
      )}
    </div>
  );
}

function summaryText(bands: RunsBands): string {
  const total = bands['needs-you'].length + bands.active.length + bands.finished.length;
  const parts = [
    total === 1 ? '1 run' : `${total} runs`,
    `${bands.active.length + bands['needs-you'].length} active`,
    `${bands['needs-you'].length} need you`,
  ];
  return parts.join(' · ');
}

interface RunsLoadedProps {
  bands: RunsBands;
  shown: RunsShown;
  instanceId?: string;
  featureId?: string;
}

function RunsLoaded({ bands, shown, instanceId, featureId }: RunsLoadedProps) {
  const { fleet } = useFleet();
  const now = Math.floor(Date.now() / 1000);
  const visible = BANDS.filter(({ band }) => shown === 'all' || shown === band);
  const listed = visible.flatMap(({ band }) => bands[band]);
  const everyRun = BANDS.flatMap(({ band }) => bands[band]);
  // A run chosen by the URL stays open when a filter hides its row.
  const selected =
    everyRun.find(({ run }) => run.instanceId === instanceId && run.featureId === featureId) ?? listed[0] ?? null;
  const selectedKey = selected === null ? null : runKey(selected);
  const scopes = fleet?.find((instance) => instance.id === selected?.run.instanceId)?.scopes ?? null;
  return (
    <div className="grid items-start gap-4 lg:grid-cols-[minmax(18rem,24rem)_minmax(0,1fr)]">
      <nav aria-label="Runs list" className="panel flex flex-col gap-4 p-2">
        {visible.map(({ band, heading, empty }) =>
          shown === 'all' && bands[band].length === 0 ? null : (
            <RunBand
              key={band}
              heading={heading}
              empty={empty}
              runs={bands[band]}
              now={now}
              selectedKey={selectedKey}
            />
          ),
        )}
      </nav>
      {selected !== null && <RunPreview key={selectedKey} listed={selected} scopes={scopes} now={now} />}
    </div>
  );
}

function runKey({ run }: ListedRun): string {
  return `${run.instanceId}/${run.featureId}`;
}

interface RunBandProps {
  heading: string;
  empty: string;
  runs: ListedRun[];
  now: number;
  selectedKey: string | null;
}

function RunBand({ heading, empty, runs, now, selectedKey }: RunBandProps) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="flex flex-col gap-1">
      <h2 id={headingId} className="px-3 pt-2 pb-1 text-xs font-medium text-slate-400">
        {heading}
      </h2>
      {runs.length === 0 ? (
        <p className="px-3 pb-2 text-sm text-slate-400">{empty}</p>
      ) : (
        <ul aria-label={heading} className="flex flex-col gap-1">
          {runs.map((listed) => (
            <li key={runKey(listed)}>
              <RunRow listed={listed} now={now} selected={runKey(listed) === selectedKey} />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
