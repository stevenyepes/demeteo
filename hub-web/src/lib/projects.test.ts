import { afterEach, describe, expect, it, vi } from 'vitest';

import { dropSession, HubApiError, setCsrfToken } from './api';
import { createProject, listProjects, type ProjectFields, removeProject, updateProject } from './projects';

type Exchange = { method: 'GET' | 'POST'; path: string; answer: Response };

/** A request that is not the next one scripted throws, so a call this file did not expect fails the test. */
function stubFetch(script: Exchange[]) {
  const pending = [...script];
  const sent: unknown[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const next = pending.shift();
      const method = init?.method ?? 'GET';
      if (next === undefined || next.path !== input || next.method !== method) {
        throw new Error(`fetch double was not told about ${method} ${String(input)} here`);
      }
      sent.push(typeof init?.body === 'string' ? (JSON.parse(init.body) as unknown) : undefined);
      return next.answer;
    }),
  );
  return { sent, pending };
}

function answer(method: 'GET' | 'POST', path: string, body: unknown, status = 200): Exchange {
  return {
    method,
    path,
    answer: new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } }),
  };
}

const ID = '0b7c8d9e-1f20-4a3b-8c4d-5e6f708192a3';

const WIRE = {
  id: ID,
  name: 'widgets',
  provider_kind: 'github',
  host: 'github.com',
  full_name: 'acme/widgets',
  remote_url: 'https://github.com/acme/widgets',
  runner_ready: true,
  default_agent_kind: 'codex',
  default_model: null,
  default_effort: 'high',
  default_workflow_id: 'wf-starter-feature',
  default_max_budget_cents: 250,
  created_at: 1_762_400_000,
  updated_at: 1_762_400_100,
};

const PROJECT = {
  id: ID,
  name: 'widgets',
  providerKind: 'github',
  host: 'github.com',
  fullName: 'acme/widgets',
  remoteUrl: 'https://github.com/acme/widgets',
  runnerReady: true,
  defaultAgentKind: 'codex',
  defaultModel: null,
  defaultEffort: 'high',
  defaultWorkflowId: 'wf-starter-feature',
  defaultMaxBudgetCents: 250,
  createdAt: 1_762_400_000,
  updatedAt: 1_762_400_100,
};

const FIELDS: ProjectFields = {
  name: 'widgets',
  providerKind: 'github',
  host: 'github.com',
  fullName: 'acme/widgets',
  defaultAgentKind: null,
  defaultModel: null,
  defaultEffort: 'low',
  defaultWorkflowId: null,
  defaultMaxBudgetCents: null,
};

const BODY = {
  name: 'widgets',
  provider_kind: 'github',
  host: 'github.com',
  full_name: 'acme/widgets',
  default_agent_kind: null,
  default_model: null,
  default_effort: 'low',
  default_workflow_id: null,
  default_max_budget_cents: null,
};

afterEach(() => {
  vi.unstubAllGlobals();
  setCsrfToken(null);
  dropSession();
});

describe('listProjects', () => {
  it('reads the projects into camelCase', async () => {
    stubFetch([answer('GET', '/v1/projects', { projects: [WIRE] })]);

    await expect(listProjects()).resolves.toEqual([PROJECT]);
  });

  it('reads an effort this build does not know as unset rather than refusing the list', async () => {
    stubFetch([answer('GET', '/v1/projects', { projects: [{ ...WIRE, default_effort: 'ultra' }] })]);

    expect((await listProjects())[0]?.defaultEffort).toBeNull();
  });

  it.each([
    ['a missing runner_ready', { runner_ready: undefined }],
    ['a numeric host', { host: 7 }],
    ['a string budget', { default_max_budget_cents: '250' }],
  ])('refuses a project with %s', async (_case, change) => {
    stubFetch([answer('GET', '/v1/projects', { projects: [{ ...WIRE, ...change }] })]);

    await expect(listProjects()).rejects.toBeInstanceOf(HubApiError);
  });
});

describe('createProject and updateProject', () => {
  it('send every field in snake_case, the unset ones as null', async () => {
    setCsrfToken('csrf-1');
    const { sent } = stubFetch([
      answer('POST', '/v1/projects', WIRE, 201),
      answer('POST', `/v1/projects/${ID}`, WIRE),
    ]);

    await expect(createProject(FIELDS)).resolves.toEqual(PROJECT);
    await expect(updateProject(ID, FIELDS)).resolves.toEqual(PROJECT);
    expect(sent).toEqual([BODY, BODY]);
  });

  it('carries the code of a refused save', async () => {
    setCsrfToken('csrf-1');
    stubFetch([answer('POST', '/v1/projects', { error: 'project_exists' }, 409)]);

    await expect(createProject(FIELDS)).rejects.toMatchObject({ status: 409, code: 'project_exists' });
  });
});

describe('removeProject', () => {
  it("takes the Hub's 204 as removed", async () => {
    setCsrfToken('csrf-1');
    const { pending } = stubFetch([
      { method: 'POST', path: `/v1/projects/${ID}/remove`, answer: new Response(null, { status: 204 }) },
    ]);

    await removeProject(ID);
    expect(pending).toHaveLength(0);
  });

  it('refuses a 200 with a body, which is not what the Hub answers', async () => {
    setCsrfToken('csrf-1');
    stubFetch([answer('POST', `/v1/projects/${ID}/remove`, { removed: true })]);

    await expect(removeProject(ID)).rejects.toBeInstanceOf(HubApiError);
  });
});
