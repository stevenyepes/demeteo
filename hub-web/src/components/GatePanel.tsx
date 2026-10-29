import { useEffect, useId, useRef, useState } from 'react';

import { HubApiError } from '../lib/api';
import type { HubScope } from '../lib/fleet';
import { decideGate, type GateDecision } from '../lib/gates';
import type { RequestTicket } from '../lib/requests';
import { CeremonyError } from './CeremonyError';
import { Button } from '../../../src/components/ui/Button';

interface GatePanelProps {
  instanceId: string;
  featureId: string;
  /** What the decision names — never the workflow node id. */
  stepExecutionId: string;
  /** `null` leaves the decision to the Hub, whose `409` is rendered. */
  scopes: HubScope[] | null;
  /** This gate's sent decision has no outcome yet, so another would hide which one the instance answered. */
  pending: boolean;
  /**
   * The queued decision's ticket. Its status is not shown here: the mirror
   * drops the gate, and this panel with it, once the instance acts on it.
   */
  onDecided: (ticket: RequestTicket) => void;
}

type Send = { state: 'idle' } | { state: 'sending' } | { state: 'failed'; error: unknown };

/** The Hub refused a challenge that is now spent or was never issued; only a fresh one can go. */
const START_AGAIN: readonly (string | null)[] = ['ceremony_expired', 'challenge_mismatch', 'too_many_ceremonies'];

/**
 * Every decision goes through `decideGate`'s passkey ceremony: there is no
 * gate kind that skips it. Redirect is shown so its absence is explained, not
 * hidden — the Hub accepts only approve and cancel. Feedback has no field: an
 * instance discards it (`docs/HUB.md` §8), so the Hub refuses a decision that
 * names it.
 */
export function GatePanel({ instanceId, featureId, stepExecutionId, scopes, pending, onDecided }: GatePanelProps) {
  const headingId = useId();
  const reasonId = useId();
  const redirectNoteId = useId();
  const pendingNoteId = useId();
  const [send, setSend] = useState<Send>({ state: 'idle' });

  /**
   * Held outside React state so two clicks inside one render still start one
   * ceremony: each spends a challenge, and a second would hide which was refused.
   */
  const inFlight = useRef(false);
  /** Bumped on unmount, so a ceremony that settles after it has no panel to land on. */
  const latestSend = useRef(0);
  useEffect(
    () => () => {
      latestSend.current += 1;
    },
    [],
  );

  const blocked = scopes !== null && !scopes.includes('gates');
  const sending = send.state === 'sending';

  const decide = (decision: GateDecision) => {
    if (blocked || pending || inFlight.current) return;
    inFlight.current = true;
    latestSend.current += 1;
    const attempt = latestSend.current;
    setSend({ state: 'sending' });
    decideGate({ instanceId, featureId, stepExecutionId, decision }).then(
      (ticket) => {
        inFlight.current = false;
        // Reported even after unmount: the decision is queued whether or not this panel is still open.
        onDecided(ticket);
        if (latestSend.current === attempt) setSend({ state: 'idle' });
      },
      (error: unknown) => {
        inFlight.current = false;
        if (latestSend.current === attempt) setSend({ state: 'failed', error });
      },
    );
  };

  return (
    <section aria-labelledby={headingId} className="mt-3 border-t border-white/10 pt-3">
      <h3 id={headingId} className="font-heading text-base font-semibold text-white">
        Gate
      </h3>
      <p className="mt-1 text-xs text-slate-400">Every gate decision asks for your passkey.</p>
      {blocked && (
        <p id={reasonId} className="mt-2 text-sm text-cyan-300">
          Deciding a gate needs the <span className="font-mono">gates</span> scope, which this instance has not
          granted.
        </p>
      )}
      {pending && !blocked && (
        <p id={pendingNoteId} className="mt-2 text-sm text-cyan-300">
          Your decision is waiting for the instance. You can decide again if it is refused or expires.
        </p>
      )}
      <fieldset
        disabled={blocked || sending || pending}
        aria-describedby={blocked ? reasonId : pending ? pendingNoteId : undefined}
        className="mt-3 flex flex-col gap-3 disabled:opacity-40"
      >
        <p className="text-xs text-slate-400">Feedback can only be given on the desktop.</p>
        <div className="flex flex-wrap items-center gap-3">
          <Button variant="primary" onClick={() => decide('approve')}>
            Approve
          </Button>
          <Button variant="secondary" onClick={() => decide('cancel')}>
            Cancel
          </Button>
          <Button variant="secondary"
            disabled
            aria-describedby={redirectNoteId}>
            Redirect
          </Button>
          <span id={redirectNoteId} className="text-xs text-slate-400">
            Redirect is not available from the Hub yet.
          </span>
        </div>
      </fieldset>
      {sending && (
        <p role="status" className="mt-3 text-sm text-cyan-300">
          Waiting for your passkey and the Hub…
        </p>
      )}
      {send.state === 'failed' && (
        <div className="mt-3 flex flex-col gap-1">
          <CeremonyError error={send.error} />
          {codeOf(send.error) === 'scope_not_granted' && (
            <p className="text-sm text-slate-300">
              The Hub says this instance has not granted the <span className="font-mono">gates</span> scope.
            </p>
          )}
          {START_AGAIN.includes(codeOf(send.error)) && (
            <p className="text-sm text-slate-300">That challenge cannot be used again. Start again to decide.</p>
          )}
        </div>
      )}
    </section>
  );
}

function codeOf(error: unknown): string | null {
  return error instanceof HubApiError ? error.code : null;
}
