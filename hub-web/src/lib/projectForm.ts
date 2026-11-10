import type { EffortLevel } from '../../../src/lib/effortLevels';
import { parseMaxCostCents, usdOfCents } from './dispatchForm';
import type { HubProject, ProjectFields } from './projects';

export const MAX_NAME_CHARS = 200;

export const PROVIDERS = ['github', 'gitlab'] as const;

export type Provider = (typeof PROVIDERS)[number];

/** The Projects form as typed. Blank defaults are unset; the remote is one pasted URL. */
export type ProjectDraft = {
  name: string;
  providerKind: Provider;
  remote: string;
  agentKind: string;
  model: string;
  effort: EffortLevel | '';
  workflowId: string;
  maxCostUsd: string;
};

export type ProjectErrors = { name?: string; remote?: string; cost?: string };

export type ProjectVerdict = { ok: true; fields: ProjectFields } | { ok: false; errors: ProjectErrors };

/**
 * `{host, fullName}` of an `https://host/owner/repo` URL, the host lowercased,
 * or `null` for any other shape. Mirrors `parse_remote` in
 * `crates/demeteo-hub/src/projects/verdict.rs`, which judges the pair again:
 * a nested group and a port are kept whole; userinfo, whitespace, control
 * characters, an empty segment and a trailing slash are refused, not repaired,
 * because a runner clones the URL as given.
 */
export function parseRemote(url: string): { host: string; fullName: string } | null {
  if (!url.startsWith('https://')) return null;
  const rest = url.slice('https://'.length);
  const slash = rest.indexOf('/');
  if (slash <= 0) return null;
  const fullName = rest.slice(slash + 1);
  const wellFormed = fullName.includes('/') && fullName.split('/').every((segment) => segment !== '');
  // biome-ignore lint/suspicious/noControlCharactersInRegex: the Hub refuses control characters, so this must too.
  if (!wellFormed || /[@\s\u0000-\u001f\u007f-\u009f]/.test(rest)) return null;
  return { host: rest.slice(0, slash).toLowerCase(), fullName };
}

/**
 * A blank form for `null`, else `project` as the form edits it. A default
 * workflow none of `workflowIds` names is blank: it reads as unset, and the
 * Hub would refuse it on save.
 */
export function draftOf(project: HubProject | null, workflowIds: readonly string[]): ProjectDraft {
  if (project === null) {
    return { name: '', providerKind: 'github', remote: '', agentKind: '', model: '', effort: '', workflowId: '', maxCostUsd: '' };
  }
  return {
    name: project.name,
    providerKind: isProvider(project.providerKind) ? project.providerKind : 'github',
    remote: project.remoteUrl,
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
export function projectVerdict(draft: ProjectDraft): ProjectVerdict {
  const name = draft.name.trim();
  const remote = parseRemote(draft.remote.trim());
  const cost = draft.maxCostUsd.trim() === '' ? null : parseMaxCostCents(draft.maxCostUsd);
  const errors: ProjectErrors = {
    ...(name === '' ? { name: 'A name is required.' } : {}),
    ...([...name].length > MAX_NAME_CHARS ? { name: `Keep the name to ${MAX_NAME_CHARS} characters.` } : {}),
    ...(name.includes('\u0000') ? { name: 'The name cannot hold a NUL character.' } : {}),
    ...(remote === null ? { remote: 'Paste the repository as https://host/owner/repo.' } : {}),
    ...(cost !== null && !cost.ok ? { cost: cost.error } : {}),
  };
  if (remote === null || errors.name !== undefined || (cost !== null && !cost.ok)) return { ok: false, errors };
  return {
    ok: true,
    fields: {
      name,
      providerKind: draft.providerKind,
      host: remote.host,
      fullName: remote.fullName,
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

function isProvider(value: string): value is Provider {
  return PROVIDERS.some((provider) => provider === value);
}
