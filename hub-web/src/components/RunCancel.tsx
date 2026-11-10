import { useState } from 'react';

import { Button } from '../../../src/components/ui/Button';
import type { HubScope } from '../lib/fleet';
import { cancelRun, type RequestTicket } from '../lib/requests';
import type { RunStatus } from '../lib/runs';
import { isScopeNotGranted } from '../lib/useRunDetail';
import { CeremonyError } from './CeremonyError';
import { InstanceSettingsHelp } from './InstanceSettingsHelp';
import { RequestStatus } from './RequestStatus';

interface RunCancelProps {
  instanceId: string;
  featureId: string;
  status: RunStatus;
  /** `null` while unknown: the button stays enabled and the Hub's `409` is rendered. */
  scopes: HubScope[] | null;
}

type Send =
  | { state: 'idle' }
  | { state: 'confirming' }
  | { state: 'sending' }
  | { state: 'sent'; ticket: RequestTicket }
  | { state: 'failed'; error: unknown };

/**
 * A sent request stays on screen after the run stops being `running`, which
 * its own acceptance makes happen: hiding it then would hide the answer.
 */
export function RunCancel({ instanceId, featureId, status, scopes }: RunCancelProps) {
  const [send, setSend] = useState<Send>({ state: 'idle' });
  const spendOff = scopes !== null && !scopes.includes('spend');

  if (send.state === 'sent') return <RequestStatus instanceId={instanceId} ticket={send.ticket} />;
  if (status !== 'running') return null;

  const confirm = () => {
    setSend({ state: 'sending' });
    cancelRun(instanceId, featureId).then(
      (ticket) => setSend({ state: 'sent', ticket }),
      (error: unknown) => setSend({ state: 'failed', error }),
    );
  };

  if (send.state === 'confirming' || send.state === 'sending') {
    const sending = send.state === 'sending';
    return (
      <div role="group" aria-label="Confirm cancel" className="flex flex-wrap items-center gap-3">
        <span className="text-sm text-slate-300">Cancel this run? Every step stops.</span>
        <Button variant="danger" size="sm" onClick={confirm} disabled={sending}>
          {sending ? 'Cancelling…' : 'Cancel run'}
        </Button>
        <Button variant="ghost" size="sm" onClick={() => setSend({ state: 'idle' })} disabled={sending}>
          Keep running
        </Button>
      </div>
    );
  }

  return (
    <div className="flex flex-col items-end gap-2">
      <Button variant="danger-subtle" size="sm" onClick={() => setSend({ state: 'confirming' })} disabled={spendOff}>
        Cancel run
      </Button>
      {spendOff && (
        <div className="flex max-w-md flex-col gap-2">
          <p className="text-right text-xs text-slate-400">
            Spend is off on this instance, so it would refuse a cancel sent from the Hub.
          </p>
          <InstanceSettingsHelp kind={null} need="spend" />
        </div>
      )}
      {send.state === 'failed' && (
        <div className="flex max-w-sm flex-col gap-1">
          <CeremonyError error={send.error} />
          {isScopeNotGranted(send.error) && (
            <>
              <p className="text-sm text-slate-300">The Hub says spend is off on this instance.</p>
              <InstanceSettingsHelp kind={null} need="spend" />
            </>
          )}
        </div>
      )}
    </div>
  );
}
