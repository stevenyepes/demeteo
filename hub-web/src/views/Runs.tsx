import { useEffect, useRef, useState } from 'react';

import { CeremonyError } from '../components/CeremonyError';
import { PlaceholderCard } from '../components/PlaceholderCard';
import { elapsedText } from '../lib/fleetSummary';
import { routeHref } from '../lib/router';
import { listRuns, type RunStatus, type RunSummary } from '../lib/runs';

type Load = { state: 'loading' } | { state: 'loaded'; runs: RunSummary[] } | { state: 'failed'; error: unknown };

type Filter = RunStatus | 'all';

const FILTERS: { filter: Filter; label: string }[] = [
  { filter: 'all', label: 'All' },
  { filter: 'running', label: 'Running' },
  { filter: 'completed', label: 'Completed' },
  { filter: 'failed', label: 'Failed' },
  { filter: 'cancelled', label: 'Cancelled' },
];

const STATUS_CLASSES: Record<RunStatus, string> = {
  running: 'text-emerald-300',
  completed: 'text-slate-300',
  failed: 'text-ruby-300',
  cancelled: 'text-slate-400',
};

export function Runs() {
  const [load, setLoad] = useState<Load>({ state: 'loading' });
  const [filter, setFilter] = useState<Filter>('all');

  /** Bumped on unmount, so a read that settles after it has no page to land on. */
  const latestRead = useRef(0);

  useEffect(() => {
    latestRead.current += 1;
    const read = latestRead.current;
    listRuns().then(
      (runs) => {
        if (latestRead.current === read) setLoad({ state: 'loaded', runs: newestFirst(runs) });
      },
      (error: unknown) => {
        if (latestRead.current === read) setLoad({ state: 'failed', error });
      },
    );
    return () => {
      latestRead.current += 1;
    };
  }, []);

  return (
    <PlaceholderCard title="Runs">
      {load.state === 'loading' && <p>Loading runs across your fleet…</p>}
      {load.state === 'failed' && (
        <div className="max-w-sm">
          <CeremonyError error={load.error} />
        </div>
      )}
      {load.state === 'loaded' && load.runs.length === 0 && <p>No runs reported by your instances yet.</p>}
      {load.state === 'loaded' && load.runs.length > 0 && (
        <Loaded runs={load.runs} filter={filter} onFilter={setFilter} />
      )}
      {load.state === 'loaded' && (
        <p className="mt-3 text-xs">Runs of instances with read off are not listed.</p>
      )}
    </PlaceholderCard>
  );
}

function newestFirst(runs: RunSummary[]): RunSummary[] {
  return [...runs].sort((a, b) => b.updatedAt - a.updatedAt);
}

interface LoadedProps {
  runs: RunSummary[];
  filter: Filter;
  onFilter: (filter: Filter) => void;
}

function Loaded({ runs, filter, onFilter }: LoadedProps) {
  const now = Math.floor(Date.now() / 1000);
  const shown = filter === 'all' ? runs : runs.filter((run) => run.status === filter);
  return (
    <div className="flex flex-col gap-3">
      <div role="group" aria-label="Status" className="flex flex-wrap gap-1">
        {FILTERS.map(({ filter: option, label }) => (
          <button
            key={option}
            type="button"
            aria-pressed={option === filter}
            onClick={() => onFilter(option)}
            className={`rounded-lg border px-3 py-1 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-cyan-400 ${
              option === filter
                ? 'border-accent-edge bg-accent-tint text-white'
                : 'border-transparent text-slate-400 hover:bg-white/5 hover:text-cyan-300'
            }`}
          >
            {label}
          </button>
        ))}
      </div>
      {shown.length === 0 ? (
        <p>No {filter} runs.</p>
      ) : (
        <ul className="nested-card flex flex-col gap-2 px-2 py-2">
          {shown.map((run) => (
            <li key={`${run.instanceId}/${run.featureId}`}>
              <RunRow run={run} now={now} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function RunRow({ run, now }: { run: RunSummary; now: number }) {
  return (
    <a
      href={routeHref({ name: 'run', instanceId: run.instanceId, featureId: run.featureId })}
      className="flex flex-wrap items-baseline gap-x-4 gap-y-1 rounded-lg px-2 py-1 transition-colors hover:bg-cyan-500/10 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400"
    >
      <span className="font-mono text-slate-200 break-all">{run.featureId}</span>
      <span className={`text-xs ${STATUS_CLASSES[run.status]}`}>{run.status}</span>
      <span className="font-mono text-xs break-all">{run.instanceId}</span>
      {run.startedAt !== null && <span className="text-xs">Started {elapsedText(run.startedAt, now)} ago</span>}
      <span className="text-xs">Updated {elapsedText(run.updatedAt, now)} ago</span>
      {typeof run.costCents === 'number' && <span className="text-xs">${(run.costCents / 100).toFixed(2)}</span>}
    </a>
  );
}
