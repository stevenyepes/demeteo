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
  settings: ProjectSettings;
  stepOverrides: ProjectStepOverride[];
  /** Unix seconds. */
  createdAt: number;
  /** Unix seconds. */
  updatedAt: number;
};

/**
 * What a run started from the project uses in place of what the runner
 * detects in the clone. `null` keeps the runner's own answer; nothing here
 * clears one. There is deliberately no gate autonomy: a Hub run's gates are
 * decided with a passkey, never by a project setting.
 */
export type ProjectSettings = {
  branchPrefix: string | null;
  /** Repo-relative. */
  artifactSubdir: string | null;
  commitArtifacts: boolean | null;
  reviewEntrypoint: string | null;
  testCommand: string | null;
  buildCommand: string | null;
  coverageCommand: string | null;
  /** Repo-relative. */
  conventionsFile: string | null;
  loopIterations: number | null;
  syncResolverAgentKind: string | null;
  syncResolverModel: string | null;
  syncResolverEffort: EffortLevel | null;
  /** `false` publishes a resolved sync without waiting for a review; `true` is never sent, as it changes nothing. */
  syncReviewBeforePush: boolean | null;
};

export const NO_SETTINGS: ProjectSettings = {
  branchPrefix: null,
  artifactSubdir: null,
  commitArtifacts: null,
  reviewEntrypoint: null,
  testCommand: null,
  buildCommand: null,
  coverageCommand: null,
  conventionsFile: null,
  loopIterations: null,
  syncResolverAgentKind: null,
  syncResolverModel: null,
  syncResolverEffort: null,
  syncReviewBeforePush: null,
};

/** A default harness for one agent step of one Hub Workflow, used by every dispatch of it that overrides nothing there itself. */
export type ProjectStepOverride = {
  workflowId: string;
  stepId: string;
  agentKind: string | null;
  model: string | null;
  effort: EffortLevel | null;
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
  settings: ProjectSettings;
  stepOverrides: ProjectStepOverride[];
};

type WireSettings = {
  branch_prefix: string | null;
  artifact_subdir: string | null;
  commit_artifacts: boolean | null;
  review_entrypoint: string | null;
  test_command: string | null;
  build_command: string | null;
  coverage_command: string | null;
  conventions_file: string | null;
  loop_iterations: number | null;
  sync_resolver_agent_kind: string | null;
  sync_resolver_model: string | null;
  sync_resolver_effort: string | null;
  sync_review_before_push: boolean | null;
};

type WireStepOverride = {
  workflow_id: string;
  step_id: string;
  agent_kind: string | null;
  model: string | null;
  effort: string | null;
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
  settings: WireSettings;
  step_overrides: WireStepOverride[];
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
    settings: wireSettingsOf(fields.settings),
    step_overrides: fields.stepOverrides.map((entry) => ({
      workflow_id: entry.workflowId,
      step_id: entry.stepId,
      agent_kind: entry.agentKind,
      model: entry.model,
      effort: entry.effort,
    })),
  };
}

function wireSettingsOf(settings: ProjectSettings): WireSettings {
  return {
    branch_prefix: settings.branchPrefix,
    artifact_subdir: settings.artifactSubdir,
    commit_artifacts: settings.commitArtifacts,
    review_entrypoint: settings.reviewEntrypoint,
    test_command: settings.testCommand,
    build_command: settings.buildCommand,
    coverage_command: settings.coverageCommand,
    conventions_file: settings.conventionsFile,
    loop_iterations: settings.loopIterations,
    sync_resolver_agent_kind: settings.syncResolverAgentKind,
    sync_resolver_model: settings.syncResolverModel,
    sync_resolver_effort: settings.syncResolverEffort,
    sync_review_before_push: settings.syncReviewBeforePush,
  };
}

function settingsOf(wire: WireSettings): ProjectSettings {
  return {
    branchPrefix: wire.branch_prefix,
    artifactSubdir: wire.artifact_subdir,
    commitArtifacts: wire.commit_artifacts,
    reviewEntrypoint: wire.review_entrypoint,
    testCommand: wire.test_command,
    buildCommand: wire.build_command,
    coverageCommand: wire.coverage_command,
    conventionsFile: wire.conventions_file,
    loopIterations: wire.loop_iterations,
    syncResolverAgentKind: wire.sync_resolver_agent_kind,
    syncResolverModel: wire.sync_resolver_model,
    syncResolverEffort: isEffortLevel(wire.sync_resolver_effort) ? wire.sync_resolver_effort : null,
    syncReviewBeforePush: wire.sync_review_before_push,
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
    settings: settingsOf(wire.settings),
    stepOverrides: wire.step_overrides.map((entry) => ({
      workflowId: entry.workflow_id,
      stepId: entry.step_id,
      agentKind: entry.agent_kind,
      model: entry.model,
      effort: isEffortLevel(entry.effort) ? entry.effort : null,
    })),
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
  isWireSettings(memberOf(value, 'settings')) &&
  isWireStepOverrides(memberOf(value, 'step_overrides')) &&
  typeof memberOf(value, 'created_at') === 'number' &&
  typeof memberOf(value, 'updated_at') === 'number';

const isWireProjects: Guard<{ projects: WireProject[] }> = (value): value is { projects: WireProject[] } => {
  const projects = memberOf(value, 'projects');
  return Array.isArray(projects) && projects.every(isWireProject);
};

const SETTINGS_TEXT = [
  'branch_prefix',
  'artifact_subdir',
  'review_entrypoint',
  'test_command',
  'build_command',
  'coverage_command',
  'conventions_file',
  'sync_resolver_agent_kind',
  'sync_resolver_model',
  'sync_resolver_effort',
] as const;

function isWireSettings(value: unknown): value is WireSettings {
  return (
    typeof value === 'object' &&
    value !== null &&
    SETTINGS_TEXT.every((key) => isStringOrNull(memberOf(value, key))) &&
    isBooleanOrNull(memberOf(value, 'commit_artifacts')) &&
    isBooleanOrNull(memberOf(value, 'sync_review_before_push')) &&
    isNumberOrNull(memberOf(value, 'loop_iterations'))
  );
}

function isWireStepOverrides(value: unknown): value is WireStepOverride[] {
  return (
    Array.isArray(value) &&
    value.every(
      (entry) =>
        typeof memberOf(entry, 'workflow_id') === 'string' &&
        typeof memberOf(entry, 'step_id') === 'string' &&
        isStringOrNull(memberOf(entry, 'agent_kind')) &&
        isStringOrNull(memberOf(entry, 'model')) &&
        isStringOrNull(memberOf(entry, 'effort')),
    )
  );
}

function isBooleanOrNull(value: unknown): value is boolean | null {
  return value === null || typeof value === 'boolean';
}

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
