import { useId } from 'react';

import { nodeTypeMeta } from '../../../src/components/canvas/types';
import { Chip } from '../../../src/components/ui/Chip';
import { runStatusMeta, TONE_TEXT } from '../../../src/lib/runStatus';
import type { HubScope } from '../lib/fleet';
import type { RequestTicket } from '../lib/requests';
import type { RunDetail } from '../lib/runs';
import { AssignmentForm } from './AssignmentForm';
import { GatePanel } from './GatePanel';
import { RequestStatus } from './RequestStatus';
import { Button } from '../../../src/components/ui/Button';

/**
 * A gate decision the Hub queued; the run holds these by the gate's
 * `step_execution_id`. Until `settled`, the gate's actions stay held: a second
 * decision would replace this one's status before its outcome is known.
 */
export type GateDecisionTicket = { stepId: string; ticket: RequestTicket; settled: boolean };

interface StepInspectorProps {
  instanceId: string;
  featureId: string;
  stepId: string;
  detail: Pick<RunDetail, 'status' | 'workflow' | 'steps' | 'pendingGates'>;
  /** The instance's granted scopes; `null` while they load or when they could not be read. */
  scopes: HubScope[] | null;
  /**
   * Held by the run, not the gate panel: the mirror drops a gate once the
   * instance acts on its decision, which is exactly when `accepted` arrives.
   */
  gateDecisions: Readonly<Record<string, GateDecisionTicket>>;
  onGateDecided: (stepExecutionId: string, decided: GateDecisionTicket) => void;
  onGateSettled: (requestId: string) => void;
  onClose: () => void;
}

/**
 * Mirror fields only. The Hub holds no transcript, log, artifact or diff
 * (docs/HUB.md §6.4), so there is nothing here to open — and the desktop
 * `NodePanel` is not reused: it loads step attempts and artifacts through the
 * desktop's own wrappers and renders the live agent stream.
 */
export function StepInspector({
  instanceId,
  featureId,
  stepId,
  detail,
  scopes,
  gateDecisions,
  onGateDecided,
  onGateSettled,
  onClose,
}: StepInspectorProps) {
  const headingId = useId();
  const decisionHeadingId = useId();
  const node = detail.workflow?.nodes.find((candidate) => candidate.id === stepId);
  const step = detail.steps.find((candidate) => candidate.stepId === stepId);
  const gate = detail.pendingGates.find((candidate) => candidate.stepId === stepId);
  const decision = heldDecision(gateDecisions, stepId, gate?.stepExecutionId);
  const state = step === undefined ? null : runStatusMeta(step.state);
  const title = node?.title ?? step?.title ?? gate?.stepTitle ?? null;
  return (
    <section aria-labelledby={headingId} className="panel p-5 text-sm text-slate-400">
      <div className="flex items-center gap-3">
        <h2 id={headingId} className="font-heading text-base font-semibold text-white">
          Step
        </h2>
        {step !== undefined && state !== null && <Chip status={step.state}>{state.label}</Chip>}
        <Button variant="secondary" size="sm" onClick={onClose} className="ml-auto">
          Close
        </Button>
      </div>
      <dl aria-label="Step details" className="mt-3 grid grid-cols-[max-content_1fr] gap-x-6 gap-y-1">
        <dt>Id</dt>
        <dd className="font-mono text-slate-200 break-all">{stepId}</dd>
        <dt>State</dt>
        <dd className={state === null ? undefined : TONE_TEXT[state.tone]}>
          {state === null ? 'not reported' : state.label}
        </dd>
        {node !== undefined && (
          <>
            <dt>Kind</dt>
            <dd className="text-slate-200">{nodeTypeMeta(node.type).label}</dd>
          </>
        )}
        {title !== null && (
          <>
            <dt>Title</dt>
            <dd className="text-slate-200 break-words">{title}</dd>
          </>
        )}
        {gate !== undefined && (
          <>
            <dt>Pending gate</dt>
            <dd className="font-mono text-slate-200 break-all">{gate.stepExecutionId}</dd>
          </>
        )}
      </dl>
      <AssignmentForm
        key={stepId}
        instanceId={instanceId}
        featureId={featureId}
        stepId={stepId}
        runStatus={detail.status}
        state={step?.state ?? null}
        kind={node?.type ?? null}
        scopes={scopes}
      />
      {gate !== undefined && (
        <GatePanel
          key={gate.stepExecutionId}
          instanceId={instanceId}
          featureId={featureId}
          stepExecutionId={gate.stepExecutionId}
          scopes={scopes}
          pending={decision !== undefined && !decision.settled}
          onDecided={(ticket) => onGateDecided(gate.stepExecutionId, { stepId, ticket, settled: false })}
        />
      )}
      {decision !== undefined && (
        <section aria-labelledby={decisionHeadingId} className="mt-4 border-t border-hairline pt-4">
          <h3 id={decisionHeadingId} className="font-heading text-base font-semibold text-white">
            Gate decision
          </h3>
          <div className="mt-2">
            <RequestStatus
              key={decision.ticket.requestId}
              instanceId={instanceId}
              ticket={decision.ticket}
              onSettled={() => onGateSettled(decision.ticket.requestId)}
            />
          </div>
        </section>
      )}
    </section>
  );
}

/**
 * While a gate is pending only its own decision is shown, so a decision on an
 * earlier execution of the step never reads as the answer to a fresh gate.
 */
function heldDecision(
  decisions: Readonly<Record<string, GateDecisionTicket>>,
  stepId: string,
  pendingExecutionId: string | undefined,
): GateDecisionTicket | undefined {
  if (pendingExecutionId !== undefined) return decisions[pendingExecutionId];
  const onStep = Object.values(decisions).filter((decided) => decided.stepId === stepId);
  return onStep[onStep.length - 1];
}
