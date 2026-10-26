import { type EffortLevel, isEffortLevel } from '../../../src/lib/effortLevels';
import { type Guard, getJson, rejectSyntheticInstance } from './api';
import { isWireRunSummary, type RunStatus, type RunSummary, runSummaryOf, type WireRunSummary } from './runs';

export type { RunStatus, RunSummary };

export type HubScope = 'read' | 'spend' | 'gates';

export type NeedsYou = {
  instanceId: string;
  featureId: string;
  stepId: string | null;
  stepExecutionId: string;
  since: number;
};

export type FleetInstance = {
  id: string;
  label: string | null;
  present: boolean;
  os: string | null;
  arch: string | null;
  version: string | null;
  scopes: HubScope[];
  readGranted: boolean;
  lastSeen: number | null;
  /** `null` is withheld without `read` (docs/HUB.md §6.4), never a count of zero. */
  liveRuns: number | null;
  needsYou: NeedsYou[] | null;
};

export type ProjectSummary = { id: string; name: string; remoteUrl: string | null };

export type WorkflowGraphNode = {
  id: string;
  type: string;
  title: string;
  position: { x: number; y: number } | null;
};

export type WorkflowChoice = {
  workflowId: string;
  version: number;
  name: string;
  graph: { nodes: WorkflowGraphNode[]; edges: { from: string; to: string }[] } | null;
};

export type RunShapeDefaults = {
  projectId: string;
  agentKind: string | null;
  model: string | null;
  effort: EffortLevel | null;
  workflowId: string | null;
  maxBudgetCents: number | null;
};

export type InstanceDetail = {
  id: string;
  present: boolean;
  connectedAt: number | null;
  lastSeen: number | null;
  installId: string | null;
  os: string | null;
  arch: string | null;
  version: string | null;
  scopes: HubScope[];
  readGranted: boolean;
  /** `null` is withheld without `read` (docs/HUB.md §6.4), never an empty list. */
  projects: ProjectSummary[] | null;
  liveRuns: RunSummary[] | null;
  harnesses: null;
  workflows: WorkflowChoice[] | null;
  runShape: RunShapeDefaults[] | null;
};

type WireNeedsYou = {
  instance_id: string;
  feature_id: string;
  step_id: string | null;
  step_execution_id: string;
  since: number;
};

type WireFleetInstance = {
  id: string;
  label: string | null;
  present: boolean;
  os: string | null;
  arch: string | null;
  version: string | null;
  scopes: unknown[];
  read_granted: boolean;
  last_seen: number | null;
  live_runs: number | null;
  needs_you: WireNeedsYou[] | null;
};

type WireProject = { id: string; name: string; remote_url: string | null };

type WireGraphNode = {
  id: string;
  type: string;
  title: string;
  position: { x: number; y: number } | null;
};

type WireGraphEdge = { from: string; to: string };

type WireWorkflow = {
  workflow_id: string;
  version: number;
  name: string;
  graph: { nodes: WireGraphNode[]; edges: WireGraphEdge[] } | null;
};

type WireRunShape = {
  project_id: string;
  default_agent_kind: string | null;
  default_model: string | null;
  /** Any string: an effort this build does not know maps to `null` rather than refusing the answer. */
  default_effort: string | null;
  default_workflow_id: string | null;
  default_max_budget_cents: number | null;
};

type WireInstanceDetail = {
  id: string;
  present: boolean;
  connected_at: number | null;
  last_seen: number | null;
  install_id: string | null;
  os: string | null;
  arch: string | null;
  version: string | null;
  scopes: unknown[];
  read_granted: boolean;
  projects: WireProject[] | null;
  live_runs: WireRunSummary[] | null;
  harnesses: null;
  workflows: WireWorkflow[] | null;
  run_shape: WireRunShape[] | null;
};

const SCOPES: readonly unknown[] = ['read', 'spend', 'gates'] satisfies HubScope[];

export async function getFleet(): Promise<FleetInstance[]> {
  const { instances } = await getJson('/v1/fleet', isFleet);
  return instances.map(fleetInstanceOf);
}

export async function getInstance(instanceId: string): Promise<InstanceDetail> {
  const path = `/v1/instances/${encodeURIComponent(instanceId)}`;
  rejectSyntheticInstance(instanceId, path);
  return instanceDetailOf(await getJson(path, isWireInstanceDetail));
}

function fleetInstanceOf(wire: WireFleetInstance): FleetInstance {
  return {
    id: wire.id,
    label: wire.label,
    present: wire.present,
    os: wire.os,
    arch: wire.arch,
    version: wire.version,
    scopes: scopesOf(wire.scopes),
    readGranted: wire.read_granted,
    lastSeen: wire.last_seen,
    liveRuns: wire.live_runs,
    needsYou: wire.needs_you === null ? null : wire.needs_you.map(needsYouOf),
  };
}

function needsYouOf(wire: WireNeedsYou): NeedsYou {
  return {
    instanceId: wire.instance_id,
    featureId: wire.feature_id,
    stepId: wire.step_id,
    stepExecutionId: wire.step_execution_id,
    since: wire.since,
  };
}

function instanceDetailOf(wire: WireInstanceDetail): InstanceDetail {
  return {
    id: wire.id,
    present: wire.present,
    connectedAt: wire.connected_at,
    lastSeen: wire.last_seen,
    installId: wire.install_id,
    os: wire.os,
    arch: wire.arch,
    version: wire.version,
    scopes: scopesOf(wire.scopes),
    readGranted: wire.read_granted,
    projects: wire.projects === null ? null : wire.projects.map(projectOf),
    liveRuns: wire.live_runs === null ? null : wire.live_runs.map(runSummaryOf),
    harnesses: null,
    workflows: wire.workflows === null ? null : wire.workflows.map(workflowOf),
    runShape: wire.run_shape === null ? null : wire.run_shape.map(runShapeOf),
  };
}

function workflowOf(wire: WireWorkflow): WorkflowChoice {
  return {
    workflowId: wire.workflow_id,
    version: wire.version,
    name: wire.name,
    graph:
      wire.graph === null
        ? null
        : {
            nodes: wire.graph.nodes.map((node) => ({
              id: node.id,
              type: node.type,
              title: node.title,
              position: node.position === null ? null : { x: node.position.x, y: node.position.y },
            })),
            edges: wire.graph.edges.map((edge) => ({ from: edge.from, to: edge.to })),
          },
  };
}

function runShapeOf(wire: WireRunShape): RunShapeDefaults {
  return {
    projectId: wire.project_id,
    agentKind: wire.default_agent_kind,
    model: wire.default_model,
    effort: isEffortLevel(wire.default_effort) ? wire.default_effort : null,
    workflowId: wire.default_workflow_id,
    maxBudgetCents: wire.default_max_budget_cents,
  };
}

function projectOf(wire: WireProject): ProjectSummary {
  return { id: wire.id, name: wire.name, remoteUrl: wire.remote_url };
}

function scopesOf(scopes: unknown[]): HubScope[] {
  return scopes.filter((scope): scope is HubScope => SCOPES.includes(scope));
}

const isWireNeedsYou: Guard<WireNeedsYou> = (value): value is WireNeedsYou =>
  typeof memberOf(value, 'instance_id') === 'string' &&
  typeof memberOf(value, 'feature_id') === 'string' &&
  isStringOrNull(memberOf(value, 'step_id')) &&
  typeof memberOf(value, 'step_execution_id') === 'string' &&
  typeof memberOf(value, 'since') === 'number';

const isWireFleetInstance: Guard<WireFleetInstance> = (value): value is WireFleetInstance =>
  typeof memberOf(value, 'id') === 'string' &&
  isStringOrNull(memberOf(value, 'label')) &&
  typeof memberOf(value, 'present') === 'boolean' &&
  isStringOrNull(memberOf(value, 'os')) &&
  isStringOrNull(memberOf(value, 'arch')) &&
  isStringOrNull(memberOf(value, 'version')) &&
  Array.isArray(memberOf(value, 'scopes')) &&
  typeof memberOf(value, 'read_granted') === 'boolean' &&
  isNumberOrNull(memberOf(value, 'last_seen')) &&
  isNumberOrNull(memberOf(value, 'live_runs')) &&
  isListOrNull(memberOf(value, 'needs_you'), isWireNeedsYou);

const isFleet: Guard<{ instances: WireFleetInstance[] }> = (
  value,
): value is { instances: WireFleetInstance[] } => {
  const instances = memberOf(value, 'instances');
  return Array.isArray(instances) && instances.every(isWireFleetInstance);
};

const isWireProject: Guard<WireProject> = (value): value is WireProject =>
  typeof memberOf(value, 'id') === 'string' &&
  typeof memberOf(value, 'name') === 'string' &&
  isStringOrNull(memberOf(value, 'remote_url'));

const isWirePosition: Guard<{ x: number; y: number }> = (value): value is { x: number; y: number } =>
  typeof memberOf(value, 'x') === 'number' && typeof memberOf(value, 'y') === 'number';

const isWireGraphNode: Guard<WireGraphNode> = (value): value is WireGraphNode => {
  const position = memberOf(value, 'position');
  return (
    typeof memberOf(value, 'id') === 'string' &&
    typeof memberOf(value, 'type') === 'string' &&
    typeof memberOf(value, 'title') === 'string' &&
    (position === null || isWirePosition(position))
  );
};

const isWireGraphEdge: Guard<WireGraphEdge> = (value): value is WireGraphEdge =>
  typeof memberOf(value, 'from') === 'string' && typeof memberOf(value, 'to') === 'string';

const isWireGraph: Guard<{ nodes: WireGraphNode[]; edges: WireGraphEdge[] }> = (
  value,
): value is { nodes: WireGraphNode[]; edges: WireGraphEdge[] } => {
  const nodes = memberOf(value, 'nodes');
  const edges = memberOf(value, 'edges');
  return Array.isArray(nodes) && nodes.every(isWireGraphNode) && Array.isArray(edges) && edges.every(isWireGraphEdge);
};

const isWireWorkflow: Guard<WireWorkflow> = (value): value is WireWorkflow => {
  const graph = memberOf(value, 'graph');
  return (
    typeof memberOf(value, 'workflow_id') === 'string' &&
    typeof memberOf(value, 'version') === 'number' &&
    typeof memberOf(value, 'name') === 'string' &&
    (graph === null || isWireGraph(graph))
  );
};

const isWireRunShape: Guard<WireRunShape> = (value): value is WireRunShape =>
  typeof memberOf(value, 'project_id') === 'string' &&
  isStringOrNull(memberOf(value, 'default_agent_kind')) &&
  isStringOrNull(memberOf(value, 'default_model')) &&
  isStringOrNull(memberOf(value, 'default_effort')) &&
  isStringOrNull(memberOf(value, 'default_workflow_id')) &&
  isNumberOrNull(memberOf(value, 'default_max_budget_cents'));

const isWireInstanceDetail: Guard<WireInstanceDetail> = (value): value is WireInstanceDetail =>
  typeof memberOf(value, 'id') === 'string' &&
  typeof memberOf(value, 'present') === 'boolean' &&
  isNumberOrNull(memberOf(value, 'connected_at')) &&
  isNumberOrNull(memberOf(value, 'last_seen')) &&
  isStringOrNull(memberOf(value, 'install_id')) &&
  isStringOrNull(memberOf(value, 'os')) &&
  isStringOrNull(memberOf(value, 'arch')) &&
  isStringOrNull(memberOf(value, 'version')) &&
  Array.isArray(memberOf(value, 'scopes')) &&
  typeof memberOf(value, 'read_granted') === 'boolean' &&
  isListOrNull(memberOf(value, 'projects'), isWireProject) &&
  isListOrNull(memberOf(value, 'live_runs'), isWireRunSummary) &&
  memberOf(value, 'harnesses') === null &&
  isListOrNull(memberOf(value, 'workflows'), isWireWorkflow) &&
  isListOrNull(memberOf(value, 'run_shape'), isWireRunShape);

function isStringOrNull(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
}

function isNumberOrNull(value: unknown): value is number | null {
  return value === null || typeof value === 'number';
}

function isListOrNull<T>(value: unknown, guard: Guard<T>): value is T[] | null {
  return value === null || (Array.isArray(value) && value.every(guard));
}

function memberOf(value: unknown, member: string): unknown {
  return typeof value === 'object' && value !== null ? Reflect.get(value, member) : undefined;
}
