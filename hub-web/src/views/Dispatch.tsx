import { useEffect, useId, useRef, useState } from 'react';

import { CeremonyError } from '../components/CeremonyError';
import { DispatchForm } from '../components/DispatchForm';
import { DispatchTargetList } from '../components/DispatchTargetList';
import { PageHeader } from '../components/PageHeader';
import { dispatchBlocker, dispatchSources } from '../lib/dispatchForm';
import { getInstance, type InstanceDetail } from '../lib/fleet';
import { useFleet } from '../lib/fleetContext';
import { routeHref } from '../lib/router';

interface DispatchProps {
  instanceId?: string;
}

type Load<T> = { state: 'loading' } | { state: 'loaded'; value: T } | { state: 'failed'; error: unknown };

/**
 * Settles `read` into `setLoad`, unless a newer read or an unmount has bumped
 * `latestRead` first, as `Instance.tsx` does: the answer would land on a page
 * that is no longer asking for it. Returns the effect's cleanup.
 */
function settle<T>(
  read: Promise<T>,
  latestRead: { current: number },
  setLoad: (load: Load<T>) => void,
): () => void {
  latestRead.current += 1;
  const attempt = latestRead.current;
  setLoad({ state: 'loading' });
  read.then(
    (value) => {
      if (latestRead.current === attempt) setLoad({ state: 'loaded', value });
    },
    (error: unknown) => {
      if (latestRead.current === attempt) setLoad({ state: 'failed', error });
    },
  );
  return () => {
    latestRead.current += 1;
  };
}

export function Dispatch({ instanceId }: DispatchProps) {
  const [chosenId, setChosenId] = useState<string | null>(null);

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        eyebrow={
          <a href={routeHref({ name: 'fleet' })} className="transition-colors hover:text-white">
            Fleet
          </a>
        }
        title="Dispatch"
        summary="Start a feature on one of your instances"
      />
      {instanceId === undefined ? (
        <div className="grid items-start gap-6 lg:grid-cols-[minmax(16rem,22rem)_minmax(0,1fr)]">
          <div className="panel p-4">
            <Targets selectedId={chosenId} onSelect={setChosenId} />
          </div>
          {chosenId === null ? (
            <p className="nested-card px-4 py-3 text-sm text-slate-400">
              Choose an instance to see its projects, workflows and pipeline.
            </p>
          ) : (
            <Target key={chosenId} instanceId={chosenId} />
          )}
        </div>
      ) : (
        <Target key={instanceId} instanceId={instanceId} />
      )}
    </div>
  );
}

/** The shell's fleet read, which the rail's badge already asked for; a second read here would be the same answer. */
function Targets({ selectedId, onSelect }: { selectedId: string | null; onSelect: (id: string) => void }) {
  const { fleet, failure } = useFleet();

  if (fleet === null && failure === null) return <p className="text-sm text-slate-400">Loading instances…</p>;
  if (fleet === null) {
    return (
      <div className="flex max-w-xl flex-col gap-2">
        <p className="text-sm text-slate-400">The fleet could not be loaded.</p>
        <CeremonyError error={failure?.error} />
      </div>
    );
  }
  return <DispatchTargetList instances={fleet} selectedId={selectedId} onSelect={onSelect} />;
}

function Target({ instanceId }: { instanceId: string }) {
  const [instance, setInstance] = useState<Load<InstanceDetail>>({ state: 'loading' });
  const latestRead = useRef(0);

  useEffect(() => settle(getInstance(instanceId), latestRead, setInstance), [instanceId]);

  const id = <span className="font-mono text-slate-200 break-all">{instanceId}</span>;

  if (instance.state === 'loading') return <p className="text-sm text-slate-400">Loading instance {id}…</p>;
  if (instance.state === 'failed') {
    return (
      <div className="flex max-w-xl flex-col gap-2">
        <p className="text-sm text-slate-400">Instance {id} could not be loaded.</p>
        <CeremonyError error={instance.error} />
      </div>
    );
  }

  const detail = instance.value;
  if (detail.kind === 'runner') return <RunnerTarget runner={detail} />;
  const blocker = dispatchBlocker(detail);
  if (blocker === 'no-read') {
    return (
      <p className="nested-card px-4 py-3 text-sm text-slate-400">
        Read is off on this instance, so the Hub cannot see its projects or workflows. Turn on read for this Hub on
        the machine to dispatch to it.
      </p>
    );
  }
  if (blocker === 'no-snapshot') {
    return (
      <p className="nested-card px-4 py-3 text-sm text-slate-400">
        This instance has reported no projects or workflows yet. They appear here once it connects and sends them.
      </p>
    );
  }
  return <DispatchForm instance={detail} spendGranted={blocker !== 'no-spend'} />;
}

/**
 * A runner holds no Projects or Workflows of its own, so a start to it names
 * the desktop whose snapshot the Hub resolves it from (docs/HUB.md §6.4). One
 * such desktop is chosen for the user; with several, nothing is until they pick.
 */
function RunnerTarget({ runner }: { runner: InstanceDetail }) {
  const pickerId = useId();
  const { fleet, failure } = useFleet();
  const sources = dispatchSources(fleet ?? []);
  const [pickedId, setPickedId] = useState<string | null>(null);
  const sourceId = pickedId ?? (sources.length === 1 ? (sources[0]?.id ?? null) : null);
  const [source, setSource] = useState<Load<InstanceDetail> | null>(null);
  const latestRead = useRef(0);

  useEffect(() => {
    if (sourceId === null) {
      latestRead.current += 1;
      setSource(null);
      return undefined;
    }
    return settle(getInstance(sourceId), latestRead, setSource);
  }, [sourceId]);

  if (fleet === null) {
    if (failure === null) return <p className="text-sm text-slate-400">Loading desktops…</p>;
    return (
      <div className="flex max-w-xl flex-col gap-2">
        <p className="text-sm text-slate-400">The fleet could not be loaded, so no desktop can be offered to run from.</p>
        <CeremonyError error={failure.error} />
      </div>
    );
  }
  if (sources.length === 0) {
    return (
      <p className="nested-card px-4 py-3 text-sm text-slate-400">
        A runner runs a project a desktop reported, and no paired desktop is reporting any with read on. Pair a desktop,
        or turn on read for this Hub on one, to dispatch here.
      </p>
    );
  }

  return (
    <div className="flex min-w-0 flex-col gap-4">
      <div className="panel flex flex-col gap-1.5 p-4">
        <label htmlFor={pickerId} className="text-xs font-medium text-slate-300">
          Run from
        </label>
        <select
          id={pickerId}
          value={sourceId ?? ''}
          onChange={(event) => setPickedId(event.target.value === '' ? null : event.target.value)}
          aria-describedby={`${pickerId}-hint`}
          className="input-field"
        >
          {sourceId === null && <option value="">Choose a desktop</option>}
          {sources.map((desktop) => (
            <option key={desktop.id} value={desktop.id}>
              {desktop.label ?? desktop.id}
            </option>
          ))}
        </select>
        <p id={`${pickerId}-hint`} className="text-xs text-slate-400">
          The runner clones the project and runs the workflow this desktop last reported.
        </p>
      </div>
      <RunnerSource runner={runner} source={source} />
    </div>
  );
}

function RunnerSource({ runner, source }: { runner: InstanceDetail; source: Load<InstanceDetail> | null }) {
  if (source === null) return null;
  if (source.state === 'loading') return <p className="text-sm text-slate-400">Loading the desktop's projects…</p>;
  if (source.state === 'failed') {
    return (
      <div className="flex max-w-xl flex-col gap-2">
        <p className="text-sm text-slate-400">The desktop could not be loaded.</p>
        <CeremonyError error={source.error} />
      </div>
    );
  }
  const blocker = dispatchBlocker(source.value);
  if (blocker === 'no-read' || blocker === 'no-snapshot') {
    return (
      <p className="nested-card px-4 py-3 text-sm text-slate-400">
        This desktop has stopped reporting its projects and workflows to the Hub. Choose another, or turn read back on
        for this Hub on it.
      </p>
    );
  }
  return (
    <DispatchForm
      key={source.value.id}
      instance={runner}
      source={source.value}
      spendGranted={runner.scopes.includes('spend')}
    />
  );
}
