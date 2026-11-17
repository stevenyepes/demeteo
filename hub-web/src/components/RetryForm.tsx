import { type FormEvent, useCallback, useEffect, useId, useRef, useState } from 'react';

import { takesAssignment } from '../../../src/lib/stepAssignment';
import { Button } from '../../../src/components/ui/Button';
import { HubApiError } from '../lib/api';
import { parseMaxCostCents } from '../lib/dispatchForm';
import type { HubScope } from '../lib/fleet';
import {
  assignmentOf,
  type RequestOutcome,
  type RequestTicket,
  retryStep,
  setRunBudget,
  setStepAssignment,
} from '../lib/requests';
import { type AssignmentDraft, AssignmentFields } from './AssignmentFields';
import { CeremonyError } from './CeremonyError';
import { RequestStatus } from './RequestStatus';

interface RetryFormProps {
  instanceId: string;
  featureId: string;
  /** The workflow node id; the instance retries its newest execution. */
  stepId: string;
  /** The node's kind, or `null` when the instance did not report the graph. */
  kind: string | null;
  /** `null` leaves the decision to the Hub, whose `409` is rendered. */
  scopes: HubScope[] | null;
  /** The run's spending cap cancelled it, so a retry on the same cap stops again at once. */
  cappedOut?: boolean;
}

/** A request the retry waits on. `cap` goes first: an assignment is no use on a run that cannot spend. */
type PriorKind = 'cap' | 'assignment';
type Prior = { kind: PriorKind; ticket: RequestTicket };

const PRIOR_LABEL: Readonly<Record<PriorKind, string>> = { cap: 'Spending cap', assignment: 'Assignment' };
const HELD_TEXT: Readonly<Record<PriorKind, string>> = {
  cap: 'The new cap did not take, so the retry was not sent.',
  assignment: 'The assignment did not take, so the retry was not sent.',
};

/**
 * `waiting` holds the rest of the chain until the newest prior is accepted:
 * the instance answers requests in no promised order, so a retry queued beside
 * a new cap or assignment could re-run the step under the one that just
 * stopped it. `held` is a prior that did not take, so nothing after it was sent.
 */
type Send =
  | { state: 'idle' }
  | { state: 'sending'; priors: Prior[] }
  | { state: 'waiting'; priors: Prior[] }
  | { state: 'held'; priors: Prior[] }
  | { state: 'sent'; priors: Prior[]; retry: RequestTicket }
  | { state: 'failed'; priors: Prior[]; error: unknown };

/** A step a failed or cancelled run stopped on: a new cap, a new assignment, then the retry. */
export function RetryForm({ instanceId, featureId, stepId, kind, scopes, cappedOut = false }: RetryFormProps) {
  const headingId = useId();
  const reasonId = useId();
  const capId = useId();
  const capErrorId = useId();
  const [draft, setDraft] = useState<AssignmentDraft>({ agent: '', model: '', effort: '' });
  const [capText, setCapText] = useState('');
  const [send, setSend] = useState<Send>({ state: 'idle' });

  /** Bumped on unmount and on each submit, so a stale answer has no form to land on. */
  const latestSend = useRef(0);
  useEffect(
    () => () => {
      latestSend.current += 1;
    },
    [],
  );

  const assignable = kind === null || takesAssignment(kind);
  const blocked = scopes !== null && !scopes.includes('spend');
  const assignment = assignable ? assignmentOf(draft.agent, draft.model, draft.effort) : null;
  const cap = cappedOut && capText.trim() !== '' ? parseMaxCostCents(capText) : null;
  const capError = cap !== null && !cap.ok ? cap.error : null;
  const busy = send.state === 'sending' || send.state === 'waiting';
  const canSend = !blocked && !busy && capError === null;

  /** Kept across the chain's steps, which run on later renders. */
  const pending = useRef<{ assignment: typeof assignment }>({ assignment: null });

  const queueRetry = useCallback(
    (attempt: number, priors: Prior[]) => {
      setSend({ state: 'sending', priors });
      retryStep(instanceId, featureId, stepId).then(
        (retry) => {
          if (latestSend.current === attempt) setSend({ state: 'sent', priors, retry });
        },
        (error: unknown) => {
          if (latestSend.current === attempt) setSend({ state: 'failed', priors, error });
        },
      );
    },
    [instanceId, featureId, stepId],
  );

  const queuePrior = useCallback(
    (attempt: number, priors: Prior[], kind: PriorKind, request: Promise<RequestTicket>) => {
      setSend({ state: 'sending', priors });
      request.then(
        (ticket) => {
          if (latestSend.current === attempt) setSend({ state: 'waiting', priors: [...priors, { kind, ticket }] });
        },
        (error: unknown) => {
          if (latestSend.current === attempt) setSend({ state: 'failed', priors, error });
        },
      );
    },
    [],
  );

  /** The next request after `priors` are all accepted: the assignment if one is left, else the retry. */
  const continueChain = useCallback(
    (attempt: number, priors: Prior[]) => {
      const left = pending.current.assignment;
      if (left === null) {
        queueRetry(attempt, priors);
        return;
      }
      pending.current.assignment = null;
      queuePrior(attempt, priors, 'assignment', setStepAssignment(instanceId, featureId, stepId, left));
    },
    [instanceId, featureId, stepId, queuePrior, queueRetry],
  );

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!canSend) return;
    latestSend.current += 1;
    const attempt = latestSend.current;
    pending.current.assignment = assignment;
    if (cap?.ok === true) queuePrior(attempt, [], 'cap', setRunBudget(instanceId, featureId, cap.value));
    else continueChain(attempt, []);
  };

  /** The status reports a settle on every render after it; the chain follows only the first. */
  const answered = useRef<string | null>(null);
  const waitingOn = send.state === 'waiting' ? send.priors[send.priors.length - 1] : undefined;
  const onPriorSettled = useCallback(
    (outcome: RequestOutcome | null) => {
      if (send.state !== 'waiting' || waitingOn === undefined || answered.current === waitingOn.ticket.requestId) {
        return;
      }
      answered.current = waitingOn.ticket.requestId;
      if (outcome?.status === 'accepted') continueChain(latestSend.current, send.priors);
      else setSend({ state: 'held', priors: send.priors });
    },
    [send, waitingOn, continueChain],
  );

  const priors = send.state === 'idle' ? [] : send.priors;
  const heldOn = send.state === 'held' ? send.priors[send.priors.length - 1] : undefined;

  return (
    <section aria-labelledby={headingId} className="mt-3 border-t border-hairline pt-3">
      <h3 id={headingId} className="font-heading text-base font-semibold text-white">
        Retry
      </h3>
      <p className="mt-1 text-xs text-slate-400">
        The instance re-runs this step and the run continues from it. Set an agent, model or effort to run the
        retry on a different assignment; it is sent first, and the retry only once the instance accepts it.
      </p>
      {cappedOut && (
        <p className="mt-2 text-sm text-slate-300">
          This run's spending cap cancelled it, and a retry under the same cap stops again at once. Raise the cap
          to go on; the instance holds the new one to its own ceilings.
        </p>
      )}
      {blocked && (
        <p id={reasonId} className="mt-2 text-sm text-slate-300">
          Retrying a step needs the <span className="font-mono">spend</span> scope, which this instance has not
          granted.
        </p>
      )}
      <form onSubmit={submit} className="mt-3">
        <fieldset
          disabled={blocked || busy}
          aria-describedby={blocked ? reasonId : undefined}
          className="flex flex-wrap items-end gap-3 disabled:opacity-40"
        >
          {cappedOut && (
            <div className="flex flex-col gap-1.5">
              <label htmlFor={capId} className="text-xs font-medium text-slate-300">
                New spending cap (USD)
              </label>
              <input
                id={capId}
                type="text"
                inputMode="decimal"
                value={capText}
                onChange={(event) => setCapText(event.target.value)}
                placeholder="Unchanged"
                autoComplete="off"
                aria-invalid={capError !== null}
                aria-describedby={capError === null ? undefined : capErrorId}
                className="input-field font-mono"
              />
            </div>
          )}
          {assignable && <AssignmentFields draft={draft} onChange={setDraft} />}
          <Button variant="primary" type="submit" disabled={!canSend}>
            {submitLabel(cap?.ok === true, assignment !== null)}
          </Button>
        </fieldset>
        {capError !== null && (
          <p id={capErrorId} className="mt-2 text-sm text-ruby-400">
            {capError}
          </p>
        )}
      </form>
      {send.state === 'sending' && (
        <p role="status" className="mt-3 text-sm text-cyan-300">
          Sending to the Hub…
        </p>
      )}
      {priors.map((prior) => (
        <div key={prior.ticket.requestId} className="mt-3 flex flex-col gap-1">
          <p className="text-xs text-slate-400">{PRIOR_LABEL[prior.kind]}</p>
          <RequestStatus
            instanceId={instanceId}
            ticket={prior.ticket}
            onSettled={prior === waitingOn ? onPriorSettled : undefined}
          />
        </div>
      ))}
      {heldOn !== undefined && <p className="mt-2 text-sm text-slate-300">{HELD_TEXT[heldOn.kind]}</p>}
      {send.state === 'sent' && (
        <div className="mt-3 flex flex-col gap-1">
          <p className="text-xs text-slate-400">Retry</p>
          <RequestStatus instanceId={instanceId} ticket={send.retry} />
        </div>
      )}
      {send.state === 'failed' && (
        <div className="mt-3 flex flex-col gap-1">
          <CeremonyError error={send.error} />
          {isScopeNotGranted(send.error) && (
            <p className="text-sm text-slate-300">
              The Hub says this instance has not granted the <span className="font-mono">spend</span> scope.
            </p>
          )}
        </div>
      )}
    </section>
  );
}

function submitLabel(raisesCap: boolean, reassigns: boolean): string {
  if (raisesCap) return reassigns ? 'Raise cap, reassign and retry' : 'Raise cap and retry';
  return reassigns ? 'Reassign and retry' : 'Retry step';
}

function isScopeNotGranted(error: unknown): boolean {
  return error instanceof HubApiError && error.code === 'scope_not_granted';
}
