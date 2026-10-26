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
  memberOf(value, 'harnesses') === null;

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
