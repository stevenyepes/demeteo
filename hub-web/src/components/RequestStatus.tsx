import { useEffect, useState } from 'react';

import { TONE_TEXT, type RunStatusTone } from '../../../src/lib/runStatus';
import { isTerminal, pollRequestStatus, type RequestOutcome, type RequestTicket } from '../lib/requests';
import { CeremonyError } from './CeremonyError';

interface RequestStatusProps {
  instanceId: string;
  ticket: RequestTicket;
  /**
   * Called once the request has an outcome, or its poll ran out at expiry
   * without one: either way nothing this request does can change any more.
   */
  onSettled?: () => void;
}

/** Only `accepted` may say the change took: until then the instance has not answered. */
const LABELS: Record<RequestOutcome['status'], string> = {
  queued: 'Queued at the Hub',
  delivered: 'Delivered, waiting for the instance',
  accepted: 'Accepted by the instance',
  refused: 'Refused by the instance',
  expired: 'Expired before the instance answered',
};

const TONES: Record<RequestOutcome['status'], RunStatusTone> = {
  queued: 'cyan',
  delivered: 'cyan',
  accepted: 'emerald',
  refused: 'ruby',
  expired: 'ruby',
};

/** The last read failed: the outcome it would have replaced is not known to be current. */
type ReadFailure = { error: unknown };

export function RequestStatus({ instanceId, ticket, onSettled }: RequestStatusProps) {
  const [outcome, setOutcome] = useState<RequestOutcome>({ status: 'queued' });
  const [expiresAt, setExpiresAt] = useState(ticket.expiresAt);
  const [failure, setFailure] = useState<ReadFailure | null>(null);
  const [stopped, setStopped] = useState(false);

  useEffect(() => {
    setOutcome({ status: 'queued' });
    setExpiresAt(ticket.expiresAt);
    setFailure(null);
    setStopped(false);
    return pollRequestStatus(
      instanceId,
      { requestId: ticket.requestId, expiresAt: ticket.expiresAt },
      (state) => {
        setOutcome(state);
        setExpiresAt(state.expiresAt);
        setFailure(null);
      },
      { onError: (error) => setFailure({ error }), onStopped: () => setStopped(true) },
    );
  }, [instanceId, ticket.requestId, ticket.expiresAt]);

  const settled = isTerminal(outcome) || stopped;
  useEffect(() => {
    if (settled) onSettled?.();
  }, [settled, onSettled]);

  const unanswered = stopped && !isTerminal(outcome);
  const expiry = new Date(expiresAt * 1000);
  return (
    <div className="flex flex-col gap-1">
      <div role="status" aria-label="Request status" className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm">
        {failure !== null ? (
          <span className="font-medium text-ruby-300">Could not read the request status</span>
        ) : unanswered ? (
          <span className="font-medium text-ruby-300">No answer from the instance before expiry</span>
        ) : (
          <span className={`font-medium ${TONE_TEXT[TONES[outcome.status]]}`}>{LABELS[outcome.status]}</span>
        )}
        {outcome.status === 'refused' && <span className="font-mono text-ruby-300 break-all">{outcome.reason}</span>}
        <span className="text-xs text-slate-400">
          {outcome.status === 'expired' || stopped ? (
            <>
              expired at <time dateTime={expiry.toISOString()}>{expiry.toLocaleTimeString()}</time>
            </>
          ) : (
            <>
              expires at <time dateTime={expiry.toISOString()}>{expiry.toLocaleTimeString()}</time>
            </>
          )}
        </span>
      </div>
      {failure !== null && <CeremonyError error={failure.error} />}
    </div>
  );
}
