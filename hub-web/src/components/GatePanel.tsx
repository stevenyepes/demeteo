import { useEffect, useId, useRef, useState } from 'react';

import { HubApiError } from '../lib/api';
import type { HubScope } from '../lib/fleet';
import { decideGate, feedbackBytes, type GateDecision, MAX_GATE_FEEDBACK_BYTES } from '../lib/gates';
import { InstanceSettingsHelp } from './InstanceSettingsHelp';
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
 * gate kind that skips it. The feedback is signed with the decision
 * (`docs/HUB.md` §8), so the instance treats it as the user's own words, as
 * the desktop's `GateView` does: notes on an approval, instructions on a
 * redirect, nothing on an abort.
 */
export function GatePanel({ instanceId, featureId, stepExecutionId, scopes, pending, onDecided }: GatePanelProps) {
  const headingId = useId();
  const reasonId = useId();
  const feedbackId = useId();
  const feedbackNoteId = useId();
  const pendingNoteId = useId();
  const [send, setSend] = useState<Send>({ state: 'idle' });
  const [feedback, setFeedback] = useState('');
  const tooLong = feedbackBytes(feedback) > MAX_GATE_FEEDBACK_BYTES;

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
    const signed = feedbackFor(decision, feedback);
    if (signed !== undefined && feedbackBytes(signed) > MAX_GATE_FEEDBACK_BYTES) return;
    inFlight.current = true;
    latestSend.current += 1;
    const attempt = latestSend.current;
    setSend({ state: 'sending' });
    decideGate({ instanceId, featureId, stepExecutionId, decision, feedback: signed }).then(
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
    <section aria-labelledby={headingId} className="mt-4 border-t border-amber-500/30 pt-4">
      <h3 id={headingId} className="font-heading text-base font-semibold text-white">
        Gate
      </h3>
      <p className="mt-1 text-xs text-slate-400">Every gate decision asks for your passkey.</p>
      {blocked && (
        <div className="mt-2 flex flex-col gap-2">
          <p id={reasonId} className="text-sm text-slate-300">
            Gates is off on this instance, so it would refuse a decision sent from the Hub.
          </p>
          <InstanceSettingsHelp kind={null} need="gates" />
        </div>
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
        <div className="flex flex-col gap-1">
          <label htmlFor={feedbackId} className="text-xs font-medium text-slate-300">
            Feedback
          </label>
          <textarea
            id={feedbackId}
            value={feedback}
            onChange={(event) => setFeedback(event.target.value)}
            rows={3}
            aria-describedby={feedbackNoteId}
            aria-invalid={tooLong}
            placeholder="What to fix, or which step to go back to."
            className="input-field disabled:cursor-not-allowed disabled:opacity-40"
          />
          <p id={feedbackNoteId} className={tooLong ? 'text-xs text-ruby-300' : 'text-xs text-slate-400'}>
            {tooLong
              ? 'Feedback is too long to send. Shorten it to 8 KB or less.'
              : 'Optional notes when you approve, instructions when you redirect. Your passkey signs it with the decision.'}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <Button variant="primary" disabled={tooLong} onClick={() => decide('approve')}>
            Approve
          </Button>
          <Button
            variant="secondary"
            disabled={tooLong}
            onClick={() => decide('redirect')}
            title="Send the run back with your feedback"
          >
            Redirect
          </Button>
          <Button variant="danger-subtle" onClick={() => decide('cancel')} title="Abort the whole feature">
            Abort feature
          </Button>
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
            <>
              <p className="text-sm text-slate-300">The Hub says gates is off on this instance.</p>
              <InstanceSettingsHelp kind={null} need="gates" />
            </>
          )}
          {START_AGAIN.includes(codeOf(send.error)) && (
            <p className="text-sm text-slate-300">That challenge cannot be used again. Start again to decide.</p>
          )}
        </div>
      )}
    </section>
  );
}

/**
 * What the passkey signs as feedback, as `GateView` sends it: an approval's
 * notes trimmed, a redirect's instructions as typed, an abort's none. Blank
 * text is no feedback, which is also how the Hub holds it.
 */
function feedbackFor(decision: GateDecision, text: string): string | undefined {
  if (decision === 'cancel' || text.trim() === '') return undefined;
  return decision === 'approve' ? text.trim() : text;
}

function codeOf(error: unknown): string | null {
  return error instanceof HubApiError ? error.code : null;
}
