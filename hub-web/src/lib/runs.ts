import type { WorkflowDefinitionV2 } from '../../../src/components/canvas/types';
import { type Guard, getJson, rejectSyntheticInstance } from './api';

export type RunStatus = 'running' | 'completed' | 'failed' | 'cancelled';

export type StepState = 'running' | 'completed' | 'failed';

export type TicketProgress = { completed: number; failed: number; running: number };

/**
 * `title`, `projectId`, `projectName` and `stepTitle` are labels from the
 * instance's snapshot, `null` when it gives none — never inferred
 * (docs/HUB.md §6.4). `stepId` is the step the run last entered.
 */
export type RunSummary = {
  instanceId: string;
  featureId: string;
  status: RunStatus;
  startedAt: number | null;
  updatedAt: number;
  costCents: number | null;
  title: string | null;
  projectId: string | null;
  projectName: string | null;
  stepId: string | null;
  stepTitle: string | null;
  tickets: TicketProgress;
};

/** Only the steps that have emitted an event; a step not yet started is not listed. */
export type StepView = { stepId: string; state: StepState; title: string | null };

/** `stepExecutionId` is what a gate decision names; `stepId` is the workflow node. */
export type PendingGate = {
  featureId: string;
  stepExecutionId: string;
  stepId: string | null;
  since: number;
  stepTitle: string | null;
};

/** `kind` is any string, shown as received, so a newer Hub's event kind does not refuse the run. */
export type RunEvent = {
  offset: number;
  kind: string;
  occurredAt: number;
  stepId: string | null;
  stepExecutionId: string | null;
  ticketId: string | null;
};

export type RunDetail = {
  instanceId: string;
  featureId: string;
  status: RunStatus;
  /** `null` is a graph the instance has not reported (docs/HUB.md §6.4) — never one to infer from `steps`. */
  workflow: WorkflowDefinitionV2 | null;
  title: string | null;
  projectId: string | null;
  projectName: string | null;
  steps: StepView[];
  tickets: TicketProgress;
  pendingGates: PendingGate[];
  events: RunEvent[];
};

export type RunsFilter = { instance?: string; status?: RunStatus };

export type WireRunSummary = {
  instance_id: string;
  feature_id: string;
  status: RunStatus;
  started_at: number | null;
  updated_at: number;
  cost_cents: number | null;
  title: string | null;
  project_id: string | null;
  project_name: string | null;
  step_id: string | null;
  step_title: string | null;
  tickets: TicketProgress;
};

type WireStepView = { step_id: string; state: StepState; title: string | null };

type WirePendingGate = {
  feature_id: string;
  step_execution_id: string;
  step_id: string | null;
  since: number;
  step_title: string | null;
};

type WireRunEvent = {
  offset: number;
  kind: string;
  occurred_at: number;
  step_id: string | null;
  step_execution_id: string | null;
  ticket_id: string | null;
};

type WireRunDetail = {
  instance_id: string;
  feature_id: string;
  status: RunStatus;
  workflow: WorkflowDefinitionV2 | null;
  title: string | null;
  project_id: string | null;
  project_name: string | null;
  steps: WireStepView[];
  tickets: TicketProgress;
  pending_gates: WirePendingGate[];
  events: WireRunEvent[];
};

const STATUSES: readonly unknown[] = ['running', 'completed', 'failed', 'cancelled'] satisfies RunStatus[];

const STEP_STATES: readonly unknown[] = ['running', 'completed', 'failed'] satisfies StepState[];

export async function listRuns(filter: RunsFilter = {}): Promise<RunSummary[]> {
  const query = new URLSearchParams();
  if (filter.instance !== undefined) query.set('instance', filter.instance);
  if (filter.status !== undefined) query.set('status', filter.status);
  const search = query.toString();
  const path = search === '' ? '/v1/runs' : `/v1/runs?${search}`;
  const { runs } = await getJson(path, isRuns);
  return runs.map(runSummaryOf);
}

export async function getRunDetail(instanceId: string, featureId: string): Promise<RunDetail> {
  const path = `/v1/instances/${encodeURIComponent(instanceId)}/runs/${encodeURIComponent(featureId)}`;
  rejectSyntheticInstance(instanceId, path);
  return runDetailOf(await getJson(path, isWireRunDetail));
}

export function runSummaryOf(wire: WireRunSummary): RunSummary {
  return {
    instanceId: wire.instance_id,
    featureId: wire.feature_id,
    status: wire.status,
    startedAt: wire.started_at,
    updatedAt: wire.updated_at,
    costCents: wire.cost_cents,
    title: wire.title,
    projectId: wire.project_id,
    projectName: wire.project_name,
    stepId: wire.step_id,
    stepTitle: wire.step_title,
    tickets: ticketProgressOf(wire.tickets),
  };
}

function ticketProgressOf(wire: TicketProgress): TicketProgress {
  return { completed: wire.completed, failed: wire.failed, running: wire.running };
}

function runDetailOf(wire: WireRunDetail): RunDetail {
  return {
    instanceId: wire.instance_id,
    featureId: wire.feature_id,
    status: wire.status,
    workflow: wire.workflow,
    title: wire.title,
    projectId: wire.project_id,
    projectName: wire.project_name,
    steps: wire.steps.map((step) => ({ stepId: step.step_id, state: step.state, title: step.title })),
    tickets: ticketProgressOf(wire.tickets),
    pendingGates: wire.pending_gates.map(pendingGateOf),
    events: wire.events.map(runEventOf),
  };
}

function pendingGateOf(wire: WirePendingGate): PendingGate {
  return {
    featureId: wire.feature_id,
    stepExecutionId: wire.step_execution_id,
    stepId: wire.step_id,
    since: wire.since,
    stepTitle: wire.step_title,
  };
}

function runEventOf(wire: WireRunEvent): RunEvent {
  return {
    offset: wire.offset,
    kind: wire.kind,
    occurredAt: wire.occurred_at,
    stepId: wire.step_id,
    stepExecutionId: wire.step_execution_id,
    ticketId: wire.ticket_id,
  };
}

export const isWireRunSummary: Guard<WireRunSummary> = (value): value is WireRunSummary =>
  typeof memberOf(value, 'instance_id') === 'string' &&
  typeof memberOf(value, 'feature_id') === 'string' &&
  STATUSES.includes(memberOf(value, 'status')) &&
  isNumberOrNull(memberOf(value, 'started_at')) &&
  typeof memberOf(value, 'updated_at') === 'number' &&
  isNumberOrNull(memberOf(value, 'cost_cents')) &&
  isStringOrNull(memberOf(value, 'title')) &&
  isStringOrNull(memberOf(value, 'project_id')) &&
  isStringOrNull(memberOf(value, 'project_name')) &&
  isStringOrNull(memberOf(value, 'step_id')) &&
  isStringOrNull(memberOf(value, 'step_title')) &&
  isTicketProgress(memberOf(value, 'tickets'));

const isRuns: Guard<{ runs: WireRunSummary[] }> = (value): value is { runs: WireRunSummary[] } =>
  isListOf(memberOf(value, 'runs'), isWireRunSummary);

const isWireStepView: Guard<WireStepView> = (value): value is WireStepView =>
  typeof memberOf(value, 'step_id') === 'string' &&
  STEP_STATES.includes(memberOf(value, 'state')) &&
  isStringOrNull(memberOf(value, 'title'));

const isTicketProgress: Guard<TicketProgress> = (value): value is TicketProgress =>
  typeof memberOf(value, 'completed') === 'number' &&
  typeof memberOf(value, 'failed') === 'number' &&
  typeof memberOf(value, 'running') === 'number';

const isWirePendingGate: Guard<WirePendingGate> = (value): value is WirePendingGate =>
  typeof memberOf(value, 'feature_id') === 'string' &&
  typeof memberOf(value, 'step_execution_id') === 'string' &&
  isStringOrNull(memberOf(value, 'step_id')) &&
  typeof memberOf(value, 'since') === 'number' &&
  isStringOrNull(memberOf(value, 'step_title'));

const isWireRunEvent: Guard<WireRunEvent> = (value): value is WireRunEvent =>
  typeof memberOf(value, 'offset') === 'number' &&
  typeof memberOf(value, 'kind') === 'string' &&
  typeof memberOf(value, 'occurred_at') === 'number' &&
  isStringOrNull(memberOf(value, 'step_id')) &&
  isStringOrNull(memberOf(value, 'step_execution_id')) &&
  isStringOrNull(memberOf(value, 'ticket_id'));

const isDrawnNode: Guard<{ id: string }> = (value): value is { id: string } =>
  typeof memberOf(value, 'id') === 'string';

const isDrawnEdge: Guard<{ from: string; to: string }> = (value): value is { from: string; to: string } =>
  typeof memberOf(value, 'from') === 'string' && typeof memberOf(value, 'to') === 'string';

/**
 * Only what the canvas reads to draw: a node's `id`, an edge's ends. The rest
 * of a definition is the desktop's to validate; a graph failing this much is a
 * failed load, never a partial drawing.
 */
const isWorkflowDefinition: Guard<WorkflowDefinitionV2> = (value): value is WorkflowDefinitionV2 =>
  isListOf(memberOf(value, 'nodes'), isDrawnNode) && isListOf(memberOf(value, 'edges'), isDrawnEdge);

const isWireRunDetail: Guard<WireRunDetail> = (value): value is WireRunDetail => {
  const workflow = memberOf(value, 'workflow');
  return (
    typeof memberOf(value, 'instance_id') === 'string' &&
    typeof memberOf(value, 'feature_id') === 'string' &&
    STATUSES.includes(memberOf(value, 'status')) &&
    (workflow === null || isWorkflowDefinition(workflow)) &&
    isStringOrNull(memberOf(value, 'title')) &&
    isStringOrNull(memberOf(value, 'project_id')) &&
    isStringOrNull(memberOf(value, 'project_name')) &&
    isListOf(memberOf(value, 'steps'), isWireStepView) &&
    isTicketProgress(memberOf(value, 'tickets')) &&
    isListOf(memberOf(value, 'pending_gates'), isWirePendingGate) &&
    isListOf(memberOf(value, 'events'), isWireRunEvent)
  );
};

function isStringOrNull(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
}

function isNumberOrNull(value: unknown): value is number | null {
  return value === null || typeof value === 'number';
}

function isListOf<T>(value: unknown, guard: Guard<T>): value is T[] {
  return Array.isArray(value) && value.every(guard);
}

function memberOf(value: unknown, member: string): unknown {
  return typeof value === 'object' && value !== null ? Reflect.get(value, member) : undefined;
}
