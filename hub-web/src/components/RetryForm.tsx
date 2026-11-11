import { type FormEvent, useCallback, useEffect, useId, useRef, useState } from 'react';

import { takesAssignment } from '../../../src/lib/stepAssignment';
import { Button } from '../../../src/components/ui/Button';
import { HubApiError } from '../lib/api';
import type { HubScope } from '../lib/fleet';
import {
  assignmentOf,
  type RequestOutcome,
  type RequestTicket,
  retryStep,
  type StepAssignment,
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
}

/**
 * `assigning` holds the retry back until the assignment is accepted: the
 * instance answers requests in no promised order, so a retry queued beside
 * it could re-run the step on the agent that just failed. `held` is an
 * assignment that did not take, so no retry was sent.
 */
type Send =
  | { state: 'idle' }
  | { state: 'sending'; assignment: RequestTicket | null }
  | { state: 'assigning'; assignment: RequestTicket }
  | { state: 'held'; assignment: RequestTicket }
  | { state: 'sent'; assignment: RequestTicket | null; retry: RequestTicket }
  | { state: 'failed'; assignment: RequestTicket | null; error: unknown };

/** A failed step of a failed run: an optional new assignment, then the retry. */
export function RetryForm({ instanceId, featureId, stepId, kind, scopes }: RetryFormProps) {
  const headingId = useId();
  const reasonId = useId();
  const [draft, setDraft] = useState<AssignmentDraft>({ agent: '', model: '', effort: '' });
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
  const busy = send.state === 'sending' || send.state === 'assigning';
  const canSend = !blocked && !busy;

  const queueRetry = useCallback(
    (attempt: number, assigned: RequestTicket | null) => {
      setSend({ state: 'sending', assignment: assigned });
      retryStep(instanceId, featureId, stepId).then(
        (retry) => {
          if (latestSend.current === attempt) setSend({ state: 'sent', assignment: assigned, retry });
        },
        (error: unknown) => {
          if (latestSend.current === attempt) setSend({ state: 'failed', assignment: assigned, error });
        },
      );
    },
    [instanceId, featureId, stepId],
  );

  const queueAssignment = (attempt: number, change: StepAssignment) => {
    setSend({ state: 'sending', assignment: null });
    setStepAssignment(instanceId, featureId, stepId, change).then(
      (ticket) => {
        if (latestSend.current === attempt) setSend({ state: 'assigning', assignment: ticket });
      },
      (error: unknown) => {
        if (latestSend.current === attempt) setSend({ state: 'failed', assignment: null, error });
      },
    );
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!canSend) return;
    latestSend.current += 1;
    const attempt = latestSend.current;
    if (assignment === null) queueRetry(attempt, null);
    else queueAssignment(attempt, assignment);
  };

  /** The status reports a settle on every render after it; the retry follows only the first. */
  const answeredAssignment = useRef<string | null>(null);
  const onAssignmentSettled = useCallback(
    (outcome: RequestOutcome | null) => {
      if (send.state !== 'assigning' || answeredAssignment.current === send.assignment.requestId) return;
      answeredAssignment.current = send.assignment.requestId;
      if (outcome?.status === 'accepted') queueRetry(latestSend.current, send.assignment);
      else setSend({ state: 'held', assignment: send.assignment });
    },
    [send, queueRetry],
  );

  const assigned = send.state === 'idle' ? null : send.assignment;

  return (
    <section aria-labelledby={headingId} className="mt-3 border-t border-hairline pt-3">
      <h3 id={headingId} className="font-heading text-base font-semibold text-white">
        Retry
      </h3>
      <p className="mt-1 text-xs text-slate-400">
        The instance re-runs this step and the run continues from it. Set an agent, model or effort to run the
        retry on a different assignment; it is sent first, and the retry only once the instance accepts it.
      </p>
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
          {assignable && <AssignmentFields draft={draft} onChange={setDraft} />}
          <Button variant="primary" type="submit" disabled={!canSend}>
            {assignment === null ? 'Retry step' : 'Reassign and retry'}
          </Button>
        </fieldset>
      </form>
      {send.state === 'sending' && (
        <p role="status" className="mt-3 text-sm text-cyan-300">
          Sending to the Hub…
        </p>
      )}
      {assigned !== null && (
        <div className="mt-3 flex flex-col gap-1">
          <p className="text-xs text-slate-400">Assignment</p>
          <RequestStatus instanceId={instanceId} ticket={assigned} onSettled={onAssignmentSettled} />
        </div>
      )}
      {send.state === 'held' && (
        <p className="mt-2 text-sm text-slate-300">The assignment did not take, so the retry was not sent.</p>
      )}
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

function isScopeNotGranted(error: unknown): boolean {
  return error instanceof HubApiError && error.code === 'scope_not_granted';
}
