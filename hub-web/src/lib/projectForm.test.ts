import { describe, expect, it } from 'vitest';

import {
  BLANK_SETTINGS,
  draftOf,
  type OverridableWorkflow,
  type ProjectDraft,
  type ProjectRepository,
  projectVerdict,
  suggestedName,
} from './projectForm';
import { type HubProject, NO_SETTINGS } from './projects';

const DRAFT: ProjectDraft = {
  name: ' widgets ',
  agentKind: '',
  model: '  ',
  effort: '',
  workflowId: '',
  maxCostUsd: '',
  settings: BLANK_SETTINGS,
  stepOverrides: [],
};

const FEATURE_WORKFLOW: OverridableWorkflow = {
  workflowId: 'wf-starter-feature',
  graph: {
    nodes: [
      { id: 's-build', type: 'agent', title: 'Build', position: null },
      { id: 's-approve', type: 'gate', title: 'Approve', position: null },
    ],
    edges: [],
  },
};

const WIDGETS_REPO: ProjectRepository = { providerKind: 'github', host: 'github.com', fullName: 'Acme/Widgets' };

describe('suggestedName', () => {
  it.each([
    ['acme/widgets', 'widgets'],
    ['group/sub/repo', 'repo'],
  ])('names %s after its last segment', (fullName, name) => {
    expect(suggestedName(fullName)).toBe(name);
  });
});

describe('projectVerdict', () => {
  it('sends the trimmed name, the parsed repository and every blank default as null', () => {
    expect(projectVerdict(DRAFT, WIDGETS_REPO)).toEqual({
      ok: true,
      fields: {
        name: 'widgets',
        providerKind: 'github',
        host: 'github.com',
        fullName: 'Acme/Widgets',
        defaultAgentKind: null,
        defaultModel: null,
        defaultEffort: null,
        defaultWorkflowId: null,
        defaultMaxBudgetCents: null,
        settings: NO_SETTINGS,
        stepOverrides: [],
      },
    });
  });

  it('reads the defaults, the budget in cents', () => {
    const verdict = projectVerdict(
      {
      ...DRAFT,
      agentKind: 'codex',
      model: 'gpt-5.5',
      effort: 'high',
      workflowId: 'wf-starter-feature',
      maxCostUsd: '2.5',
      },
      { ...WIDGETS_REPO, providerKind: 'gitlab' },
    );
    expect(verdict).toMatchObject({
      ok: true,
      fields: {
        providerKind: 'gitlab',
        defaultAgentKind: 'codex',
        defaultModel: 'gpt-5.5',
        defaultEffort: 'high',
        defaultWorkflowId: 'wf-starter-feature',
        defaultMaxBudgetCents: 250,
      },
    });
  });

  it('says why each refused field is refused', () => {
    const verdict = projectVerdict({ ...DRAFT, name: '  ', maxCostUsd: '0' }, null);
    expect(verdict.ok).toBe(false);
    if (!verdict.ok) {
      expect(Object.keys(verdict.errors).sort()).toEqual(['cost', 'name', 'repository']);
      expect(verdict.errors.repository).toBe('Choose a repository.');
    }
  });

  it('refuses a name past 200 characters or holding NUL', () => {
    expect(projectVerdict({ ...DRAFT, name: 'x'.repeat(200) }, WIDGETS_REPO).ok).toBe(true);
    expect(projectVerdict({ ...DRAFT, name: 'x'.repeat(201) }, WIDGETS_REPO)).toMatchObject({ ok: false, errors: { name: expect.any(String) } });
    expect(projectVerdict({ ...DRAFT, name: 'a\u0000b' }, WIDGETS_REPO)).toMatchObject({ ok: false, errors: { name: expect.any(String) } });
  });
});

describe('draftOf', () => {
  it('is blank for a new project', () => {
    expect(draftOf(null, [])).toEqual({
      name: '',
      agentKind: '',
      model: '',
      effort: '',
      workflowId: '',
      maxCostUsd: '',
      settings: BLANK_SETTINGS,
      stepOverrides: [],
    });
  });

  it('round-trips a stored project through the verdict unchanged', () => {
    const project: HubProject = {
      id: 'p-1',
      name: 'widgets',
      providerKind: 'gitlab',
      host: 'gitlab.example',
      fullName: 'group/sub/repo',
      remoteUrl: 'https://gitlab.example/group/sub/repo',
      runnerReady: false,
      defaultAgentKind: 'claude-code',
      defaultModel: 'opus',
      defaultEffort: 'max',
      defaultWorkflowId: 'wf-starter-feature',
      defaultMaxBudgetCents: 1_005,
      settings: {
        ...NO_SETTINGS,
        branchPrefix: 'hub/',
        commitArtifacts: true,
        testCommand: 'cargo test',
        loopIterations: 4,
        syncResolverAgentKind: 'codex',
        syncResolverEffort: 'low',
        syncReviewBeforePush: false,
      },
      stepOverrides: [{ workflowId: 'wf-starter-feature', stepId: 's-build', agentKind: 'codex', model: null, effort: null }],
      createdAt: 1,
      updatedAt: 2,
    };
    expect(projectVerdict(draftOf(project, [FEATURE_WORKFLOW]), project)).toEqual({
      ok: true,
      fields: {
        name: 'widgets',
        providerKind: 'gitlab',
        host: 'gitlab.example',
        fullName: 'group/sub/repo',
        defaultAgentKind: 'claude-code',
        defaultModel: 'opus',
        defaultEffort: 'max',
        defaultWorkflowId: 'wf-starter-feature',
        defaultMaxBudgetCents: 1_005,
        settings: project.settings,
        stepOverrides: project.stepOverrides,
      },
    });
  });

  it('blanks a default workflow this build no longer offers, so saving does not send it', () => {
    const project: HubProject = {
      id: 'p-1',
      name: 'widgets',
      providerKind: 'github',
      host: 'github.com',
      fullName: 'acme/widgets',
      remoteUrl: 'https://github.com/acme/widgets',
      runnerReady: true,
      defaultAgentKind: null,
      defaultModel: null,
      defaultEffort: null,
      defaultWorkflowId: 'wf-starter-dropped',
      defaultMaxBudgetCents: null,
      settings: NO_SETTINGS,
      stepOverrides: [],
      createdAt: 1,
      updatedAt: 2,
    };
    const draft = draftOf(project, [FEATURE_WORKFLOW]);
    expect(draft.workflowId).toBe('');
    const verdict = projectVerdict(draft, project);
    expect(verdict.ok && verdict.fields.defaultWorkflowId).toBeNull();
  });
});

describe('settings', () => {
  const withSettings = (settings: Partial<ProjectDraft['settings']>) =>
    projectVerdict({ ...DRAFT, settings: { ...BLANK_SETTINGS, ...settings } }, WIDGETS_REPO);

  it('sends blank settings as null, an unchecked box as null rather than false', () => {
    const verdict = withSettings({ testCommand: '   ', commitArtifacts: false, pushResolvedSync: false });
    expect(verdict.ok && verdict.fields.settings).toEqual(NO_SETTINGS);
  });

  it('sends a checked push-without-review as false, the one value the runner reads', () => {
    const verdict = withSettings({ pushResolvedSync: true, commitArtifacts: true });
    expect(verdict.ok && verdict.fields.settings).toMatchObject({ syncReviewBeforePush: false, commitArtifacts: true });
  });

  it.each([
    ['branchPrefix', { branchPrefix: 'my branch/' }],
    ['branchPrefix', { branchPrefix: '-f' }],
    ['branchPrefix', { branchPrefix: 'a..b/' }],
    ['artifactSubdir', { artifactSubdir: '/tmp/out' }],
    ['artifactSubdir', { artifactSubdir: 'C:\\out' }],
    ['conventionsFile', { conventionsFile: 'docs/../../etc' }],
    ['loopIterations', { loopIterations: '0' }],
    ['loopIterations', { loopIterations: '11' }],
    ['loopIterations', { loopIterations: '2.5' }],
  ])('refuses %s as the Hub would', (field, settings) => {
    const verdict = withSettings(settings);
    expect(verdict.ok).toBe(false);
    if (!verdict.ok) expect(Object.keys(verdict.errors)).toEqual([field]);
  });

  it('accepts what the Hub accepts', () => {
    const verdict = withSettings({
      branchPrefix: 'team/hub-',
      artifactSubdir: 'docs/reports/',
      conventionsFile: 'AGENTS.md',
      loopIterations: '10',
    });
    expect(verdict.ok && verdict.fields.settings).toMatchObject({
      branchPrefix: 'team/hub-',
      artifactSubdir: 'docs/reports/',
      conventionsFile: 'AGENTS.md',
      loopIterations: 10,
    });
  });
});

describe('step overrides in draftOf', () => {
  it('drops an override of a Workflow or a step this build no longer offers', () => {
    const kept = { workflowId: 'wf-starter-feature', stepId: 's-build', agentKind: 'codex', model: null, effort: null };
    const project: HubProject = {
      id: 'p-1',
      name: 'widgets',
      providerKind: 'github',
      host: 'github.com',
      fullName: 'acme/widgets',
      remoteUrl: 'https://github.com/acme/widgets',
      runnerReady: true,
      defaultAgentKind: null,
      defaultModel: null,
      defaultEffort: null,
      defaultWorkflowId: null,
      defaultMaxBudgetCents: null,
      settings: NO_SETTINGS,
      stepOverrides: [
        kept,
        { ...kept, stepId: 's-gone' },
        { ...kept, stepId: 's-approve' },
        { ...kept, workflowId: 'wf-starter-dropped' },
      ],
      createdAt: 1,
      updatedAt: 2,
    };
    expect(draftOf(project, [FEATURE_WORKFLOW]).stepOverrides).toEqual([kept]);
  });
});
