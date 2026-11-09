import { type EffortLevel, isEffortLevel } from '../../../src/lib/effortLevels';
import { type Guard, getJson } from './api';
import { postInSession } from './auth';

/**
 * A Project this Hub owns, as `GET /v1/projects` lists it: a repository a
 * runner clones and the run shape a dispatch to a runner starts from. It is
 * never a desktop's Project and is never synced with one.
 */
export type HubProject = {
  id: string;
  name: string;
  /** `github` or `gitlab`. */
  providerKind: string;
  host: string;
  /** `owner/repo`, or a nested group's whole path. */
  fullName: string;
  /** The Hub's spelling of `https://{host}/{fullName}`, the URL a runner clones. */
  remoteUrl: string;
  /** False for a project a runner cannot be dispatched against yet: GitLab today. */
  runnerReady: boolean;
  defaultAgentKind: string | null;
  defaultModel: string | null;
  defaultEffort: EffortLevel | null;
  defaultWorkflowId: string | null;
  defaultMaxBudgetCents: number | null;
  /** Unix seconds. */
  createdAt: number;
  /** Unix seconds. */
  updatedAt: number;
};

/** What a create or an update sends: every field, since an update replaces the project whole. */
export type ProjectFields = {
  name: string;
  providerKind: string;
  host: string;
  fullName: string;
  defaultAgentKind: string | null;
  defaultModel: string | null;
  defaultEffort: EffortLevel | null;
  defaultWorkflowId: string | null;
  defaultMaxBudgetCents: number | null;
};

type WireProject = {
  id: string;
  name: string;
  provider_kind: string;
  host: string;
  full_name: string;
  remote_url: string;
  runner_ready: boolean;
  default_agent_kind: string | null;
  default_model: string | null;
  default_effort: string | null;
  default_workflow_id: string | null;
  default_max_budget_cents: number | null;
  created_at: number;
  updated_at: number;
};

export async function listProjects(): Promise<HubProject[]> {
  const wire = await getJson('/v1/projects', isWireProjects);
  return wire.projects.map(projectOf);
}

export async function createProject(fields: ProjectFields): Promise<HubProject> {
  return projectOf(await postInSession('/v1/projects', bodyOf(fields), isWireProject));
}

export async function updateProject(id: string, fields: ProjectFields): Promise<HubProject> {
  return projectOf(await postInSession(`/v1/projects/${encodeURIComponent(id)}`, bodyOf(fields), isWireProject));
}

/** A run already queued or started from the project keeps what it was signed with. */
export async function removeProject(id: string): Promise<void> {
  await postInSession(`/v1/projects/${encodeURIComponent(id)}/remove`, {}, isNoContent);
}

function bodyOf(fields: ProjectFields): Record<string, unknown> {
  return {
    name: fields.name,
    provider_kind: fields.providerKind,
    host: fields.host,
    full_name: fields.fullName,
    default_agent_kind: fields.defaultAgentKind,
    default_model: fields.defaultModel,
    default_effort: fields.defaultEffort,
    default_workflow_id: fields.defaultWorkflowId,
    default_max_budget_cents: fields.defaultMaxBudgetCents,
  };
}

function projectOf(wire: WireProject): HubProject {
  return {
    id: wire.id,
    name: wire.name,
    providerKind: wire.provider_kind,
    host: wire.host,
    fullName: wire.full_name,
    remoteUrl: wire.remote_url,
    runnerReady: wire.runner_ready,
    defaultAgentKind: wire.default_agent_kind,
    defaultModel: wire.default_model,
    defaultEffort: isEffortLevel(wire.default_effort) ? wire.default_effort : null,
    defaultWorkflowId: wire.default_workflow_id,
    defaultMaxBudgetCents: wire.default_max_budget_cents,
    createdAt: wire.created_at,
    updatedAt: wire.updated_at,
  };
}

const isWireProject: Guard<WireProject> = (value): value is WireProject =>
  typeof memberOf(value, 'id') === 'string' &&
  typeof memberOf(value, 'name') === 'string' &&
  typeof memberOf(value, 'provider_kind') === 'string' &&
  typeof memberOf(value, 'host') === 'string' &&
  typeof memberOf(value, 'full_name') === 'string' &&
  typeof memberOf(value, 'remote_url') === 'string' &&
  typeof memberOf(value, 'runner_ready') === 'boolean' &&
  isStringOrNull(memberOf(value, 'default_agent_kind')) &&
  isStringOrNull(memberOf(value, 'default_model')) &&
  isStringOrNull(memberOf(value, 'default_effort')) &&
  isStringOrNull(memberOf(value, 'default_workflow_id')) &&
  isNumberOrNull(memberOf(value, 'default_max_budget_cents')) &&
  typeof memberOf(value, 'created_at') === 'number' &&
  typeof memberOf(value, 'updated_at') === 'number';

const isWireProjects: Guard<{ projects: WireProject[] }> = (value): value is { projects: WireProject[] } => {
  const projects = memberOf(value, 'projects');
  return Array.isArray(projects) && projects.every(isWireProject);
};

const isNoContent: Guard<null> = (value): value is null => value === null;

function isStringOrNull(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
}

function isNumberOrNull(value: unknown): value is number | null {
  return value === null || typeof value === 'number';
}

function memberOf(value: unknown, member: string): unknown {
  return typeof value === 'object' && value !== null ? Reflect.get(value, member) : undefined;
}
