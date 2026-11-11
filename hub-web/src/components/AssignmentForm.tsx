import { type FormEvent, useEffect, useId, useRef, useState } from 'react';

import { nodeTypeMeta } from '../../../src/components/canvas/types';
import { isAssignable, takesAssignment } from '../../../src/lib/stepAssignment';
import { HubApiError } from '../lib/api';
import type { HubScope } from '../lib/fleet';
import { assignmentOf, type RequestTicket, setStepAssignment } from '../lib/requests';
import type { RunStatus, StepState } from '../lib/runs';
import { type AssignmentDraft, AssignmentFields } from './AssignmentFields';
import { CeremonyError } from './CeremonyError';
import { RequestStatus } from './RequestStatus';
import { Button } from '../../../src/components/ui/Button';

interface AssignmentFormProps {
  instanceId: string;
  featureId: string;
  /** The workflow node id — never a `step_execution_id`. */
  stepId: string;
  runStatus: RunStatus;
  /** The mirror's state for the step, or `null` when it reported none. */
  state: StepState | null;
  /** The node's kind, or `null` when the instance did not report the graph. */
  kind: string | null;
  /** `null` leaves the decision to the Hub, whose `409` is rendered. */
  scopes: HubScope[] | null;
}

type Send = { state: 'idle' } | { state: 'sending' } | { state: 'sent'; ticket: RequestTicket } | { state: 'failed'; error: unknown };

export function AssignmentForm({ instanceId, featureId, stepId, runStatus, state, kind, scopes }: AssignmentFormProps) {
  const headingId = useId();
  const reasonId = useId();
  const [draft, setDraft] = useState<AssignmentDraft>({ agent: '', model: '', effort: '' });
  const [send, setSend] = useState<Send>({ state: 'idle' });

  /** Bumped on unmount, so a POST that settles after it has no form to land on. */
  const latestSend = useRef(0);
  useEffect(
    () => () => {
      latestSend.current += 1;
    },
    [],
  );

  const blocked = blockedBy(runStatus, state, kind, scopes);
  const assignment = assignmentOf(draft.agent, draft.model, draft.effort);
  const canSend = blocked === null && assignment !== null && send.state !== 'sending';

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!canSend) return;
    latestSend.current += 1;
    const attempt = latestSend.current;
    setSend({ state: 'sending' });
    setStepAssignment(instanceId, featureId, stepId, assignment).then(
      (ticket) => {
        if (latestSend.current === attempt) setSend({ state: 'sent', ticket });
      },
      (error: unknown) => {
        if (latestSend.current === attempt) setSend({ state: 'failed', error });
      },
    );
  };

  return (
    <section aria-labelledby={headingId} className="mt-3 border-t border-white/10 pt-3">
      <h3 id={headingId} className="font-heading text-base font-semibold text-white">
        Assignment
      </h3>
      <p className="mt-1 text-xs text-slate-400">
        The instance applies a new assignment the next time this step runs. Leave a field unchanged to keep what
        it has.
      </p>
      {blocked !== null && (
        <p id={reasonId} className="mt-2 text-sm text-slate-300">
          {blocked}
        </p>
      )}
      <form onSubmit={submit} className="mt-3">
        <fieldset
          disabled={blocked !== null}
          aria-describedby={blocked === null ? undefined : reasonId}
          className="flex flex-wrap items-end gap-3 disabled:opacity-40"
        >
          <AssignmentFields draft={draft} onChange={setDraft} />
          <Button variant="primary"
            type="submit"
            disabled={!canSend}>
            Change assignment
          </Button>
        </fieldset>
      </form>
      {send.state === 'sending' && (
        <p role="status" className="mt-3 text-sm text-cyan-300">
          Sending to the Hub…
        </p>
      )}
      {send.state === 'sent' && (
        <div className="mt-3">
          <RequestStatus instanceId={instanceId} ticket={send.ticket} />
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

function blockedBy(runStatus: RunStatus, state: StepState | null, kind: string | null, scopes: HubScope[] | null) {
  if (runStatus !== 'running') return `This run is ${runStatus}; it has no next attempt to assign.`;
  if (kind !== null && !takesAssignment(kind)) {
    return `A ${nodeTypeMeta(kind).label.toLowerCase()} step runs no agent, so it takes no assignment.`;
  }
  if (state !== null && !isAssignable(state)) {
    return 'This step is running; its assignment cannot change until it stops.';
  }
  if (scopes !== null && !scopes.includes('spend')) {
    return (
      <>
        Changing an assignment needs the <span className="font-mono">spend</span> scope, which this instance has
        not granted.
      </>
    );
  }
  return null;
}

function isScopeNotGranted(error: unknown): boolean {
  return error instanceof HubApiError && error.code === 'scope_not_granted';
}
