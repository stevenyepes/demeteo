import { useEffect, useRef, useState } from 'react';

import { HubApiError } from '../lib/api';
import { type Enqueued, getRequestStatus, type RequestStatus, refusalSentence } from '../lib/dispatch';

const FIRST_POLL_MS = 2_000;
const MAX_POLL_MS = 10_000;

const TERMINAL: ReadonlySet<RequestStatus['status']> = new Set(['accepted', 'refused', 'expired']);

const TONE = {
  waiting: 'border-cyan-500/30 bg-cyan-500/10 text-cyan-300',
  accepted: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-300',
  refused: 'border-ruby-500/30 bg-ruby-500/10 text-ruby-300',
  expired: 'border-white/10 bg-black/40 text-slate-300',
};

/**
 * `null` once past expiry, so polling stops. Otherwise the back-off, held to
 * land one poll just after `expiresAt`: the Hub only answers `expired` once the
 * request's TTL has run out, and without that poll a request that expired
 * between two back-off steps would be left reading `queued`.
 */
function nextPollDelay(attempt: number, nowMs: number, expiresAtMs: number): number | null {
  if (nowMs > expiresAtMs) return null;
  return Math.min(FIRST_POLL_MS * 2 ** attempt, MAX_POLL_MS, expiresAtMs - nowMs + FIRST_POLL_MS);
}

type Poll =
  | { state: 'polling' | 'done'; status: RequestStatus['status']; reason: string | null }
  | { state: 'failed'; error: unknown };

interface RequestStatusCardProps {
  instanceId: string;
  enqueued: Enqueued;
  /** The instance's presence when the request was sent; offline reads as queued until it reconnects. */
  present: boolean;
}

export function RequestStatusCard({ instanceId, enqueued, present }: RequestStatusCardProps) {
  const [poll, setPoll] = useState<Poll>({ state: 'polling', status: 'queued', reason: null });
  const latestRead = useRef(0);
  const { requestId, expiresAt } = enqueued;

  useEffect(() => {
    latestRead.current += 1;
    const read = latestRead.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setPoll({ state: 'polling', status: 'queued', reason: null });

    const schedule = (attempt: number) => {
      const delay = nextPollDelay(attempt, Date.now(), expiresAt * 1000);
      if (delay === null) {
        setPoll((held) => (held.state === 'polling' ? { ...held, state: 'done' } : held));
        return;
      }
      timer = setTimeout(() => {
        getRequestStatus(instanceId, requestId).then(
          (answer) => {
            if (latestRead.current !== read) return;
            const reason = answer.status === 'refused' ? answer.reason : null;
            const terminal = TERMINAL.has(answer.status);
            setPoll({ state: terminal ? 'done' : 'polling', status: answer.status, reason });
            if (!terminal) schedule(attempt + 1);
          },
          (error: unknown) => {
            if (latestRead.current === read) setPoll({ state: 'failed', error });
          },
        );
      }, delay);
    };
    schedule(0);

    return () => {
      latestRead.current += 1;
      clearTimeout(timer);
    };
  }, [instanceId, requestId, expiresAt]);

  if (poll.state === 'failed') {
    const { error } = poll;
    const code = error instanceof HubApiError ? (error.code ?? error.message) : String(error);
    return (
      <div role="alert" className={`rounded-lg border px-4 py-3 text-sm ${TONE.refused}`}>
        <p className="font-medium">The Hub stopped answering for this request's status.</p>
        <p className="font-mono text-xs break-all">{code}</p>
      </div>
    );
  }

  const expiry = new Date(expiresAt * 1000);
  const expiryTime = <time dateTime={expiry.toISOString()}>{expiry.toLocaleTimeString()}</time>;

  return (
    <div role="status" className={`rounded-lg border px-4 py-3 text-sm ${toneOf(poll.status)}`}>
      {poll.status === 'queued' && !present && (
        <>
          <p className="font-medium">Queued until the instance reconnects</p>
          <p className="text-xs text-slate-400">It expires at {expiryTime} if the instance has not taken it by then.</p>
        </>
      )}
      {poll.status === 'queued' && present && (
        <>
          <p className="font-medium">Queued</p>
          <p className="text-xs text-slate-400">Waiting for the instance to take it. It expires at {expiryTime}.</p>
        </>
      )}
      {poll.status === 'delivered' && (
        <>
          <p className="font-medium">Delivered</p>
          <p className="text-xs text-slate-400">The instance has the request and is deciding.</p>
        </>
      )}
      {poll.status === 'accepted' && <p className="font-medium">Accepted: the instance is starting the Feature.</p>}
      {poll.status === 'refused' && <Refused reason={poll.reason} />}
      {poll.status === 'expired' && (
        <p className="font-medium">Expired: the instance did not act on the request before {expiryTime}.</p>
      )}
      {poll.state === 'done' && !TERMINAL.has(poll.status) && (
        <p className="text-xs text-slate-400">Stopped checking: the request is past its expiry.</p>
      )}
    </div>
  );
}

function Refused({ reason }: { reason: string | null }) {
  const sentence = reason === null ? null : refusalSentence(reason);
  return (
    <>
      <p className="font-medium">Refused by the instance</p>
      {sentence !== null && <p>{sentence}</p>}
      {reason !== null && <p className="font-mono text-xs break-all">{reason}</p>}
    </>
  );
}

function toneOf(status: RequestStatus['status']): string {
  if (status === 'accepted') return TONE.accepted;
  if (status === 'refused') return TONE.refused;
  if (status === 'expired') return TONE.expired;
  return TONE.waiting;
}
