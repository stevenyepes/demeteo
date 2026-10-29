import { type FormEvent, useEffect, useId, useRef, useState } from 'react';

import { nodeTypeMeta } from '../../../src/components/canvas/types';
import { AGENTS } from '../../../src/lib/agents';
import { EFFORT_LABELS, EFFORT_LEVELS, isEffortLevel } from '../../../src/lib/effortLevels';
import { isAssignable, takesAssignment } from '../../../src/lib/stepAssignment';
import { HubApiError } from '../lib/api';
import type { HubScope } from '../lib/fleet';
import { type RequestTicket, type StepAssignment, setStepAssignment } from '../lib/requests';
import type { RunStatus, StepState } from '../lib/runs';
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

/**
 * The model is free text: the Hub reports no harness catalogue, and the
 * desktop's `agentModels.ts` / `agentCatalog.ts` reach Tauri.
 */
export function AssignmentForm({ instanceId, featureId, stepId, runStatus, state, kind, scopes }: AssignmentFormProps) {
  const headingId = useId();
  const reasonId = useId();
  const agentId = useId();
  const modelId = useId();
  const effortId = useId();
  const [agent, setAgent] = useState('');
  const [model, setModel] = useState('');
  const [effort, setEffort] = useState('');
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
  const assignment = assignmentOf(agent, model, effort);
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
        <p id={reasonId} className="mt-2 text-sm text-cyan-300">
          {blocked}
        </p>
      )}
      <form onSubmit={submit} className="mt-3">
        <fieldset
          disabled={blocked !== null}
          aria-describedby={blocked === null ? undefined : reasonId}
          className="flex flex-wrap items-end gap-3 disabled:opacity-40"
        >
          <div className="flex flex-col gap-1.5">
            <label htmlFor={agentId} className="text-xs font-medium text-slate-300">
              Agent
            </label>
            <select id={agentId} value={agent} onChange={(event) => setAgent(event.target.value)} className="input-field">
              <option value="">Unchanged</option>
              {Object.values(AGENTS).map((meta) => (
                <option key={meta.kind} value={meta.kind}>
                  {meta.label}
                </option>
              ))}
            </select>
          </div>
          <div className="flex flex-col gap-1.5">
            <label htmlFor={modelId} className="text-xs font-medium text-slate-300">
              Model
            </label>
            <input
              id={modelId}
              type="text"
              value={model}
              onChange={(event) => setModel(event.target.value)}
              placeholder="Unchanged"
              autoComplete="off"
              className="input-field font-mono focus:border-cyan-400! focus:shadow-none!"
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <label htmlFor={effortId} className="text-xs font-medium text-slate-300">
              Effort
            </label>
            <select id={effortId} value={effort} onChange={(event) => setEffort(event.target.value)} className="input-field">
              <option value="">Unchanged</option>
              {EFFORT_LEVELS.map((level) => (
                <option key={level} value={level}>
                  {EFFORT_LABELS[level]}
                </option>
              ))}
            </select>
          </div>
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

/** Only the fields the user set; `null` when none is. */
function assignmentOf(agent: string, model: string, effort: string): StepAssignment | null {
  const assignment: StepAssignment = {};
  if (agent !== '') assignment.agentKind = agent;
  if (model.trim() !== '') assignment.model = model.trim();
  if (isEffortLevel(effort)) assignment.effort = effort;
  return Object.keys(assignment).length === 0 ? null : assignment;
}

function isScopeNotGranted(error: unknown): boolean {
  return error instanceof HubApiError && error.code === 'scope_not_granted';
}
