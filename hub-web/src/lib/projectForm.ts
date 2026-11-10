import type { EffortLevel } from '../../../src/lib/effortLevels';
import { parseMaxCostCents, usdOfCents } from './dispatchForm';
import type { HubProject, ProjectFields } from './projects';

export const MAX_NAME_CHARS = 200;

/** The repository a project clones: picked from a connected Git host, or, while editing, the one it already has. */
export type ProjectRepository = { providerKind: string; host: string; fullName: string };

/** The Projects form as typed. Blank defaults are unset. */
export type ProjectDraft = {
  name: string;
  agentKind: string;
  model: string;
  effort: EffortLevel | '';
  workflowId: string;
  maxCostUsd: string;
};

export type ProjectErrors = { name?: string; repository?: string; cost?: string };

export type ProjectVerdict = { ok: true; fields: ProjectFields } | { ok: false; errors: ProjectErrors };

/** The name a picked repository suggests: its last path segment. */
export function suggestedName(fullName: string): string {
  return fullName.slice(fullName.lastIndexOf('/') + 1);
}

/**
 * A blank form for `null`, else `project` as the form edits it. A default
 * workflow none of `workflowIds` names is blank: it reads as unset, and the
 * Hub would refuse it on save.
 */
export function draftOf(project: HubProject | null, workflowIds: readonly string[]): ProjectDraft {
  if (project === null) {
    return { name: '', agentKind: '', model: '', effort: '', workflowId: '', maxCostUsd: '' };
  }
  return {
    name: project.name,
    agentKind: project.defaultAgentKind ?? '',
    model: project.defaultModel ?? '',
    effort: project.defaultEffort ?? '',
    workflowId: workflowIds.find((id) => id === project.defaultWorkflowId) ?? '',
    maxCostUsd: usdOfCents(project.defaultMaxBudgetCents ?? 0),
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
  const errors: ProjectErrors = {
    ...(name === '' ? { name: 'A name is required.' } : {}),
    ...([...name].length > MAX_NAME_CHARS ? { name: `Keep the name to ${MAX_NAME_CHARS} characters.` } : {}),
    ...(name.includes('\u0000') ? { name: 'The name cannot hold a NUL character.' } : {}),
    ...(repository === null ? { repository: 'Choose a repository.' } : {}),
    ...(cost !== null && !cost.ok ? { cost: cost.error } : {}),
  };
  if (repository === null || errors.name !== undefined || (cost !== null && !cost.ok)) return { ok: false, errors };
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
      defaultMaxBudgetCents: cost === null ? null : cost.value,
    },
  };
}

function blankAsNull(text: string): string | null {
  const trimmed = text.trim();
  return trimmed === '' ? null : trimmed;
}
