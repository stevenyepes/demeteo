import { useEffect, useRef, useState } from 'react';

import { CeremonyError } from '../components/CeremonyError';
import { DispatchForm } from '../components/DispatchForm';
import { DispatchTargetList } from '../components/DispatchTargetList';
import { PlaceholderCard } from '../components/PlaceholderCard';
import { dispatchBlocker } from '../lib/dispatchForm';
import { type FleetInstance, getFleet, getInstance, type InstanceDetail } from '../lib/fleet';

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
  const targetId = instanceId ?? chosenId;

  return (
    <PlaceholderCard title="Dispatch">
      <div className="flex flex-col gap-4">
        {instanceId === undefined && <Targets selectedId={chosenId} onSelect={setChosenId} />}
        {targetId !== null && <Target key={targetId} instanceId={targetId} />}
      </div>
    </PlaceholderCard>
  );
}

function Targets({ selectedId, onSelect }: { selectedId: string | null; onSelect: (id: string) => void }) {
  const [fleet, setFleet] = useState<Load<FleetInstance[]>>({ state: 'loading' });
  const latestRead = useRef(0);

  useEffect(() => settle(getFleet(), latestRead, setFleet), []);

  if (fleet.state === 'loading') return <p>Loading instances…</p>;
  if (fleet.state === 'failed') {
    return (
      <div className="flex max-w-xl flex-col gap-2">
        <p>The fleet could not be loaded.</p>
        <CeremonyError error={fleet.error} />
      </div>
    );
  }
  return <DispatchTargetList instances={fleet.value} selectedId={selectedId} onSelect={onSelect} />;
}

function Target({ instanceId }: { instanceId: string }) {
  const [instance, setInstance] = useState<Load<InstanceDetail>>({ state: 'loading' });
  const latestRead = useRef(0);

  useEffect(() => settle(getInstance(instanceId), latestRead, setInstance), [instanceId]);

  const id = <span className="font-mono text-slate-200 break-all">{instanceId}</span>;

  if (instance.state === 'loading') return <p>Loading instance {id}…</p>;
  if (instance.state === 'failed') {
    return (
      <div className="flex max-w-xl flex-col gap-2">
        <p>Instance {id} could not be loaded.</p>
        <CeremonyError error={instance.error} />
      </div>
    );
  }

  const detail = instance.value;
  const blocker = dispatchBlocker(detail);
  if (blocker === 'no-read') {
    return (
      <p className="nested-card px-4 py-3">
        Read is off on this instance, so the Hub cannot see its projects or workflows. Turn on read for this Hub on
        the machine to dispatch to it.
      </p>
    );
  }
  if (blocker === 'no-snapshot') {
    return (
      <p className="nested-card px-4 py-3">
        This instance has reported no projects or workflows yet. They appear here once it connects and sends them.
      </p>
    );
  }
  return <DispatchForm instance={detail} spendGranted={blocker !== 'no-spend'} />;
}
