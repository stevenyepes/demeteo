import { describe, expect, it } from 'vitest';

import { draftOf, type ProjectDraft, parseRemote, projectVerdict } from './projectForm';
import type { HubProject } from './projects';

const DRAFT: ProjectDraft = {
  name: ' widgets ',
  providerKind: 'github',
  remote: ' https://GitHub.com/Acme/Widgets ',
  agentKind: '',
  model: '  ',
  effort: '',
  workflowId: '',
  maxCostUsd: '',
};

describe('parseRemote', () => {
  it.each([
    ['https://github.com/acme/widgets', 'github.com', 'acme/widgets'],
    ['https://GitHub.COM/Acme/Widgets', 'github.com', 'Acme/Widgets'],
    ['https://gitlab.example:8443/group/sub/repo', 'gitlab.example:8443', 'group/sub/repo'],
  ])('reads %s', (url, host, fullName) => {
    expect(parseRemote(url)).toEqual({ host, fullName });
  });

  it.each([
    'http://github.com/acme/widgets',
    'git@github.com:acme/widgets',
    'https://github.com/widgets',
    'https:///acme/widgets',
    'https://github.com/acme//widgets',
    'https://github.com/acme/widgets/',
    'https://user@github.com/acme/widgets',
    'https://github.com/acme/wid gets',
    'https://github.com/acme/wid\u0085gets',
    'https://github.com/acme/wid\u0000gets',
    'https://github.com',
    '',
  ])('refuses %j, as the Hub would', (url) => {
    expect(parseRemote(url)).toBeNull();
  });
});

describe('projectVerdict', () => {
  it('sends the trimmed name, the parsed repository and every blank default as null', () => {
    expect(projectVerdict(DRAFT)).toEqual({
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
    const verdict = projectVerdict({
      ...DRAFT,
      providerKind: 'gitlab',
      agentKind: 'codex',
      model: 'gpt-5.5',
      effort: 'high',
      workflowId: 'wf-starter-feature',
      maxCostUsd: '2.5',
    });
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
    const verdict = projectVerdict({ ...DRAFT, name: '  ', remote: 'git@github.com:a/b', maxCostUsd: '0' });
    expect(verdict.ok).toBe(false);
    if (!verdict.ok) {
      expect(Object.keys(verdict.errors).sort()).toEqual(['cost', 'name', 'remote']);
      expect(verdict.errors.remote).toMatch(/https:\/\/host\/owner\/repo/);
    }
  });

  it('refuses a name past 200 characters or holding NUL', () => {
    expect(projectVerdict({ ...DRAFT, name: 'x'.repeat(200) }).ok).toBe(true);
    expect(projectVerdict({ ...DRAFT, name: 'x'.repeat(201) })).toMatchObject({ ok: false, errors: { name: expect.any(String) } });
    expect(projectVerdict({ ...DRAFT, name: 'a\u0000b' })).toMatchObject({ ok: false, errors: { name: expect.any(String) } });
  });
});

describe('draftOf', () => {
  it('is blank for a new project', () => {
    expect(draftOf(null, [])).toEqual({
      name: '',
      providerKind: 'github',
      remote: '',
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
    expect(projectVerdict(draftOf(project, ['wf-starter-feature']))).toEqual({
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
    const verdict = projectVerdict(draft);
    expect(verdict.ok && verdict.fields.defaultWorkflowId).toBeNull();
  });
});
