import { useEffect, useRef, useState } from 'react';

import { Button } from '../../../src/components/ui/Button';
import { useForgetInstance } from '../lib/fleetContext';
import { removalProblemText, removeInstance } from '../lib/instanceRemoval';
import { routeHref } from '../lib/router';
import { CeremonyError } from './CeremonyError';

interface RemoveInstanceProps {
  instanceId: string;
}

type Remove = { state: 'idle' } | { state: 'confirming' } | { state: 'removing' } | { state: 'failed'; error: unknown };

export function RemoveInstance({ instanceId }: RemoveInstanceProps) {
  const [remove, setRemove] = useState<Remove>({ state: 'idle' });
  const forgetInstance = useForgetInstance();

  /**
   * Held outside React state so two clicks inside one render still start one
   * ceremony: each spends a challenge, and a second would hide which was refused.
   */
  const inFlight = useRef(false);
  /** Cleared on unmount, so a ceremony that settles after it has no page to land on. */
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const confirm = () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setRemove({ state: 'removing' });
    removeInstance(instanceId).then(
      () => {
        inFlight.current = false;
        // Forgotten even after unmount: the Hub has removed it whether or not this page is still open.
        forgetInstance(instanceId);
        if (mounted.current) window.location.hash = routeHref({ name: 'fleet' });
      },
      (error: unknown) => {
        inFlight.current = false;
        if (mounted.current) setRemove({ state: 'failed', error });
      },
    );
  };

  if (remove.state === 'idle') {
    return (
      <Button variant="danger-subtle" onClick={() => setRemove({ state: 'confirming' })}>
        Remove instance
      </Button>
    );
  }

  const removing = remove.state === 'removing';
  const problem = remove.state === 'failed' ? removalProblemText(remove.error) : null;
  return (
    <div role="group" aria-label="Confirm removal" className="flex max-w-xl flex-col gap-3 text-sm text-slate-300">
      <p>
        Removing this instance revokes its pairing. If it is online it is disconnected, and it has to pair again to
        rejoin. Its run history stays on the Hub.
      </p>
      <div className="flex flex-wrap items-center gap-3">
        <Button variant="danger" size="sm" onClick={confirm} disabled={removing}>
          {removing ? 'Removing…' : 'Remove with passkey'}
        </Button>
        <Button variant="ghost" size="sm" onClick={() => setRemove({ state: 'idle' })} disabled={removing}>
          Keep instance
        </Button>
      </div>
      {removing && (
        <p role="status" className="text-slate-400">
          Waiting for your passkey and the Hub…
        </p>
      )}
      {remove.state === 'failed' && (
        <div className="flex flex-col gap-1">
          <CeremonyError error={remove.error} />
          {problem !== null && <p>{problem}</p>}
        </div>
      )}
    </div>
  );
}
