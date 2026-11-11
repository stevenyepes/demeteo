import { EFFORT_SUPPORT, type EffortLevel } from '../../../src/lib/effortLevels';
import { isOverridable, parseMaxCostCents, usdOfCents } from './dispatchForm';
import type { WorkflowChoice } from './fleet';
import type { HubProject, ProjectFields, ProjectSettings, ProjectStepOverride } from './projects';

export const MAX_NAME_CHARS = 200;

/** The bound the Hub, like the desktop, holds a project's retry loops to. */
export const LOOP_ITERATIONS = { min: 1, max: 10 } as const;

/** What the form needs of a Workflow to keep only the step overrides it can still apply. */
export type OverridableWorkflow = Pick<WorkflowChoice, 'workflowId' | 'graph'>;

/** The repository a project clones: picked from a connected Git host, or, while editing, the one it already has. */
export type ProjectRepository = { providerKind: string; host: string; fullName: string };

/** The settings as typed. Blank keeps what the runner detects or defaults to. */
export type SettingsDraft = {
  branchPrefix: string;
  artifactSubdir: string;
  /** Unchecked leaves the runner's default, which keeps reports off the branch. */
  commitArtifacts: boolean;
  reviewEntrypoint: string;
  testCommand: string;
  buildCommand: string;
  coverageCommand: string;
  conventionsFile: string;
  loopIterations: string;
  syncResolverAgentKind: string;
  syncResolverModel: string;
  syncResolverEffort: EffortLevel | '';
  /** Checked publishes a resolved sync without waiting for a review. */
  pushResolvedSync: boolean;
};

/** The Projects form as typed. Blank defaults are unset. */
export type ProjectDraft = {
  name: string;
  agentKind: string;
  model: string;
  effort: EffortLevel | '';
  workflowId: string;
  maxCostUsd: string;
  settings: SettingsDraft;
  /** Held as sent: only `ProjectStepOverrides` edits them, and it never keeps an entry with nothing set. */
  stepOverrides: ProjectStepOverride[];
};

export type ProjectErrors = {
  name?: string;
  repository?: string;
  cost?: string;
  branchPrefix?: string;
  artifactSubdir?: string;
  conventionsFile?: string;
  loopIterations?: string;
};

export const BLANK_SETTINGS: SettingsDraft = {
  branchPrefix: '',
  artifactSubdir: '',
  commitArtifacts: false,
  reviewEntrypoint: '',
  testCommand: '',
  buildCommand: '',
  coverageCommand: '',
  conventionsFile: '',
  loopIterations: '',
  syncResolverAgentKind: '',
  syncResolverModel: '',
  syncResolverEffort: '',
  pushResolvedSync: false,
};

export type ProjectVerdict = { ok: true; fields: ProjectFields } | { ok: false; errors: ProjectErrors };

/**
 * The agents a project may name, from this build's own table: a Hub project is
 * not tied to one runner, so no instance's catalog can say. A held kind the
 * table lacks is kept as its own choice rather than dropped.
 */
export function agentKindsWith(held: string): string[] {
  const known = Object.keys(EFFORT_SUPPORT);
  return held !== '' && !known.includes(held) ? [...known, held] : known;
}

/** The name a picked repository suggests: its last path segment. */
export function suggestedName(fullName: string): string {
  return fullName.slice(fullName.lastIndexOf('/') + 1);
}

/**
 * A blank form for `null`, else `project` as the form edits it. A default
 * workflow none of `workflows` names is blank, and a step override of a
 * Workflow or step this build no longer has is dropped: each reads as unset,
 * and the Hub would refuse it on save.
 */
export function draftOf(project: HubProject | null, workflows: readonly OverridableWorkflow[]): ProjectDraft {
  if (project === null) {
    return {
      name: '',
      agentKind: '',
      model: '',
      effort: '',
      workflowId: '',
      maxCostUsd: '',
      settings: BLANK_SETTINGS,
      stepOverrides: [],
    };
  }
  const workflowIds = workflows.map((workflow) => workflow.workflowId);
  return {
    name: project.name,
    agentKind: project.defaultAgentKind ?? '',
    model: project.defaultModel ?? '',
    effort: project.defaultEffort ?? '',
    workflowId: workflowIds.find((id) => id === project.defaultWorkflowId) ?? '',
    maxCostUsd: usdOfCents(project.defaultMaxBudgetCents ?? 0),
    settings: settingsDraftOf(project.settings),
    stepOverrides: project.stepOverrides.filter((entry) =>
      overridableSteps(workflows.find((workflow) => workflow.workflowId === entry.workflowId)).some(
        (step) => step.id === entry.stepId,
      ),
    ),
  };
}

/** The steps of `workflow` that run an agent, so have one to override; none for an unknown Workflow. */
export function overridableSteps(workflow: OverridableWorkflow | undefined): { id: string; type: string; title: string }[] {
  return (workflow?.graph?.nodes ?? []).filter((node) => isOverridable(node.type));
}

function settingsDraftOf(settings: ProjectSettings): SettingsDraft {
  return {
    branchPrefix: settings.branchPrefix ?? '',
    artifactSubdir: settings.artifactSubdir ?? '',
    commitArtifacts: settings.commitArtifacts === true,
    reviewEntrypoint: settings.reviewEntrypoint ?? '',
    testCommand: settings.testCommand ?? '',
    buildCommand: settings.buildCommand ?? '',
    coverageCommand: settings.coverageCommand ?? '',
    conventionsFile: settings.conventionsFile ?? '',
    loopIterations: settings.loopIterations === null ? '' : String(settings.loopIterations),
    syncResolverAgentKind: settings.syncResolverAgentKind ?? '',
    syncResolverModel: settings.syncResolverModel ?? '',
    syncResolverEffort: settings.syncResolverEffort ?? '',
    pushResolvedSync: settings.syncReviewBeforePush === false,
  };
}

/**
 * The fields to send for `draft`, or why each refused field is refused. Unlike
 * a dispatch, a blank max cost is allowed here: it leaves the project with no
 * default, and every dispatch then asks for one.
 */
export function projectVerdict(draft: ProjectDraft, repository: ProjectRepository | null): ProjectVerdict {
  const name = draft.name.trim();
  const cost = draft.maxCostUsd.trim() === '' ? null : parseMaxCostCents(draft.maxCostUsd);
  const settings = settingsVerdict(draft.settings);
  const errors: ProjectErrors = {
    ...settings.errors,
    ...(name === '' ? { name: 'A name is required.' } : {}),
    ...([...name].length > MAX_NAME_CHARS ? { name: `Keep the name to ${MAX_NAME_CHARS} characters.` } : {}),
    ...(name.includes('\u0000') ? { name: 'The name cannot hold a NUL character.' } : {}),
    ...(repository === null ? { repository: 'Choose a repository.' } : {}),
    ...(cost !== null && !cost.ok ? { cost: cost.error } : {}),
  };
  if (repository === null || Object.keys(errors).length > 0) return { ok: false, errors };
  return {
    ok: true,
    fields: {
      name,
      providerKind: repository.providerKind,
      host: repository.host,
      fullName: repository.fullName,
      defaultAgentKind: blankAsNull(draft.agentKind),
      defaultModel: blankAsNull(draft.model),
      defaultEffort: draft.effort === '' ? null : draft.effort,
      defaultWorkflowId: blankAsNull(draft.workflowId),
      defaultMaxBudgetCents: cost === null || !cost.ok ? null : cost.value,
      settings: settings.value,
      stepOverrides: draft.stepOverrides,
    },
  };
}

const PREFIX_FORBIDDEN = /[\s~^:?*[\\\p{Cc}]/u;

/**
 * The Hub's own rules (`crates/demeteo-hub/src/projects/verdict.rs`), said in
 * words here so a refusal names its field rather than coming back as one
 * `invalid_request` for the whole form.
 */
function settingsVerdict(draft: SettingsDraft): { value: ProjectSettings; errors: ProjectErrors } {
  const branchPrefix = blankAsNull(draft.branchPrefix);
  const artifactSubdir = blankAsNull(draft.artifactSubdir);
  const conventionsFile = blankAsNull(draft.conventionsFile);
  const loops = draft.loopIterations.trim();
  const loopIterations = loops === '' ? null : Number(loops);
  const loopsOk =
    loopIterations === null ||
    (Number.isInteger(loopIterations) && loopIterations >= LOOP_ITERATIONS.min && loopIterations <= LOOP_ITERATIONS.max);
  const outside = 'Name a folder inside the repository, without a leading slash, a drive or "..".';
  const errors: ProjectErrors = {
    ...(branchPrefix !== null && !branchPrefixOk(branchPrefix)
      ? { branchPrefix: 'A branch name cannot start this way: no spaces, "..", "~^:?*[\\", or a leading "-", "/" or ".".' }
      : {}),
    ...(artifactSubdir !== null && !insideRepository(artifactSubdir) ? { artifactSubdir: outside } : {}),
    ...(conventionsFile !== null && !insideRepository(conventionsFile)
      ? { conventionsFile: 'Name a file inside the repository, without a leading slash, a drive or "..".' }
      : {}),
    ...(loopsOk ? {} : { loopIterations: `A whole number from ${LOOP_ITERATIONS.min} to ${LOOP_ITERATIONS.max}.` }),
  };
  return {
    errors,
    value: {
      branchPrefix,
      artifactSubdir,
      commitArtifacts: draft.commitArtifacts ? true : null,
      reviewEntrypoint: blankAsNull(draft.reviewEntrypoint),
      testCommand: blankAsNull(draft.testCommand),
      buildCommand: blankAsNull(draft.buildCommand),
      coverageCommand: blankAsNull(draft.coverageCommand),
      conventionsFile,
      loopIterations: loopsOk ? loopIterations : null,
      syncResolverAgentKind: blankAsNull(draft.syncResolverAgentKind),
      syncResolverModel: blankAsNull(draft.syncResolverModel),
      syncResolverEffort: draft.syncResolverEffort === '' ? null : draft.syncResolverEffort,
      syncReviewBeforePush: draft.pushResolvedSync ? false : null,
    },
  };
}

function branchPrefixOk(prefix: string): boolean {
  return (
    !/^[-/.]/.test(prefix) &&
    !prefix.includes('..') &&
    !prefix.includes('//') &&
    !prefix.includes('@{') &&
    !PREFIX_FORBIDDEN.test(prefix)
  );
}

function insideRepository(relative: string): boolean {
  return (
    !/^[/\\]/.test(relative) &&
    relative[1] !== ':' &&
    !/\p{Cc}/u.test(relative) &&
    relative.split(/[/\\]/).every((segment) => segment !== '..')
  );
}

function blankAsNull(text: string): string | null {
  const trimmed = text.trim();
  return trimmed === '' ? null : trimmed;
}
