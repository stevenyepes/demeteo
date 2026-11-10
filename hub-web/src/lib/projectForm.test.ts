import { describe, expect, it } from 'vitest';

import { draftOf, type ProjectDraft, type ProjectRepository, projectVerdict, suggestedName } from './projectForm';
import type { HubProject } from './projects';

const DRAFT: ProjectDraft = {
  name: ' widgets ',
  agentKind: '',
  model: '  ',
  effort: '',
  workflowId: '',
  maxCostUsd: '',
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
      createdAt: 1,
      updatedAt: 2,
    };
    expect(projectVerdict(draftOf(project, ['wf-starter-feature']), project)).toEqual({
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
      createdAt: 1,
      updatedAt: 2,
    };
    const draft = draftOf(project, ['wf-starter-feature']);
    expect(draft.workflowId).toBe('');
    const verdict = projectVerdict(draft, project);
    expect(verdict.ok && verdict.fields.defaultWorkflowId).toBeNull();
  });
});
