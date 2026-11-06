import { afterEach, describe, expect, it, vi } from 'vitest';

import { HubApiError, setCsrfToken } from './api';
import { getFleet, getInstance } from './fleet';

type Exchange = { method: 'GET'; path: string; answer: Response };

/** Every request a double was not told about, held to `[]` after each test. */
let unscripted: string[] = [];

/** The scripted double of `auth.test.ts`: a request that is not the next one in `script` throws. */
function stubFetch(script: Exchange[]) {
  const pending = [...script];
  const sent: string[] = [];
  const double = vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
    const method = init?.method ?? 'GET';
    const next = pending.shift();
    if (next === undefined || next.path !== input || next.method !== method) {
      unscripted.push(`${method} ${String(input)}`);
      throw new Error(`fetch double was not told about ${method} ${String(input)} here`);
    }
    sent.push(next.path);
    return next.answer;
  });
  vi.stubGlobal('fetch', double);
  return { sent, pending, double };
}

function get(path: string, body: unknown, status = 200): Exchange {
  return { method: 'GET', path, answer: json(body, status) };
}

function json(body: unknown, status: number): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

async function rejectionOf(pending: Promise<unknown>): Promise<HubApiError> {
  const error: unknown = await pending.then(
    () => null,
    (reason: unknown) => reason,
  );
  if (!(error instanceof HubApiError)) {
    throw new Error(`expected a HubApiError, got ${String(error)}`);
  }
  return error;
}

const INSTANCE_ID = '7f3e2a10-9c4b-4d2e-8f1a-0b6c5d4e3f21';
const OTHER_ID = '0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d';

const WIRE_NEEDS_YOU = {
  instance_id: INSTANCE_ID,
  feature_id: 'f-1',
  step_id: 's-gate',
  step_execution_id: 'se-1',
  since: 1_700_000_100,
  title: 'Retry webhooks',
  project_name: 'checkout-api',
  step_title: 'Review gate',
};

const WIRE_FLEET_INSTANCE = {
  id: INSTANCE_ID,
  kind: 'desktop',
  label: 'laptop',
  present: true,
  os: 'linux',
  arch: 'x86_64',
  version: '1.4.0',
  scopes: ['read', 'gates'],
  read_granted: true,
  last_seen: 1_700_000_200,
  live_runs: 2,
  needs_you: [WIRE_NEEDS_YOU],
  harnesses: ['claude-code', 'opencode'],
  project_count: 3,
};

const WIRE_RUN = {
  instance_id: INSTANCE_ID,
  feature_id: 'f-1',
  status: 'running',
  started_at: 1_700_000_000,
  updated_at: 1_700_000_300,
  cost_cents: null,
  title: 'Retry webhooks',
  project_id: 'p-1',
  project_name: 'demeteo',
  step_id: 's-implement',
  step_title: 'Implement',
  tickets: { completed: 1, failed: 0, running: 2 },
};

const WIRE_PROJECT = { id: 'p-1', name: 'demeteo', remote_url: 'https://example.com/demeteo.git' };

const WIRE_NODE = { id: 's-implement', type: 'agent', title: 'Implement', position: { x: 120, y: 40 } };

const WIRE_WORKFLOW = {
  workflow_id: 'wf-feature',
  version: 3,
  name: 'Feature',
  graph: {
    nodes: [WIRE_NODE, { id: 's-gate', type: 'gate', title: 'Review', position: null }],
    edges: [{ from: 's-implement', to: 's-gate' }],
  },
};

const WIRE_RUN_SHAPE = {
  project_id: 'p-1',
  default_agent_kind: 'claude-code',
  default_model: 'opus',
  default_effort: 'xhigh',
  default_workflow_id: 'wf-feature',
  default_max_budget_cents: 500,
};

const WIRE_DETAIL = {
  id: INSTANCE_ID,
  kind: 'runner',
  present: true,
  connected_at: 1_700_000_050,
  last_seen: 1_700_000_200,
  install_id: 'install-1',
  os: 'linux',
  arch: 'x86_64',
  version: '1.4.0',
  scopes: ['read', 'spend', 'gates'],
  read_granted: true,
  projects: [WIRE_PROJECT],
  live_runs: [WIRE_RUN],
  harnesses: ['claude-code'],
  workflows: [WIRE_WORKFLOW],
  run_shape: [WIRE_RUN_SHAPE],
};

const DETAIL_PATH = `/v1/instances/${INSTANCE_ID}`;

afterEach(() => {
  vi.unstubAllGlobals();
  setCsrfToken(null);
  const strays = unscripted;
  unscripted = [];
  // Soft: a throw here would skip the setup file's own `afterEach`.
  expect.soft(strays).toEqual([]);
});

describe('getFleet', () => {
  it.each<[string, unknown]>([
    ['a kind this build does not know', 'mainframe'],
    ['null', null],
    ['no kind at all', undefined],
  ])('reads %s as an unknown kind and still shows the instance', async (_shape, kind) => {
    stubFetch([get('/v1/fleet', { instances: [{ ...WIRE_FLEET_INSTANCE, kind }] })]);

    const [instance] = await getFleet();

    expect(instance?.kind).toBeNull();
  });

  it('maps every wire instance to its view shape, in the order sent', async () => {
    const { pending } = stubFetch([
      get('/v1/fleet', {
        instances: [
          WIRE_FLEET_INSTANCE,
          {
            ...WIRE_FLEET_INSTANCE,
            id: OTHER_ID,
            kind: 'runner',
            label: 'desk',
            present: false,
            live_runs: 0,
            needs_you: [],
            harnesses: [],
            project_count: 0,
          },
        ],
      }),
    ]);

    await expect(getFleet()).resolves.toEqual([
      {
        id: INSTANCE_ID,
        kind: 'desktop',
        label: 'laptop',
        present: true,
        os: 'linux',
        arch: 'x86_64',
        version: '1.4.0',
        scopes: ['read', 'gates'],
        readGranted: true,
        lastSeen: 1_700_000_200,
        liveRuns: 2,
        needsYou: [
          {
            instanceId: INSTANCE_ID,
            featureId: 'f-1',
            stepId: 's-gate',
            stepExecutionId: 'se-1',
            since: 1_700_000_100,
            title: 'Retry webhooks',
            projectName: 'checkout-api',
            stepTitle: 'Review gate',
          },
        ],
        harnesses: ['claude-code', 'opencode'],
        projectCount: 3,
      },
      {
        id: OTHER_ID,
        kind: 'runner',
        label: 'desk',
        present: false,
        os: 'linux',
        arch: 'x86_64',
        version: '1.4.0',
        scopes: ['read', 'gates'],
        readGranted: true,
        lastSeen: 1_700_000_200,
        liveRuns: 0,
        needsYou: [],
        harnesses: [],
        projectCount: 0,
      },
    ]);
    expect(pending).toEqual([]);
  });

  it('keeps every withheld or unknown field null rather than coercing it', async () => {
    stubFetch([
      get('/v1/fleet', {
        instances: [
          {
            ...WIRE_FLEET_INSTANCE,
            label: null,
            os: null,
            arch: null,
            version: null,
            scopes: [],
            read_granted: false,
            last_seen: null,
            live_runs: null,
            needs_you: null,
            harnesses: null,
            project_count: null,
          },
        ],
      }),
    ]);

    const [instance] = await getFleet();

    expect(instance).toMatchObject({
      harnesses: null,
      projectCount: null,
      label: null,
      os: null,
      arch: null,
      version: null,
      readGranted: false,
      lastSeen: null,
      liveRuns: null,
      needsYou: null,
    });
  });

  it('keeps a needs-you entry the snapshot gives no label for unlabelled', async () => {
    stubFetch([
      get('/v1/fleet', {
        instances: [
          {
            ...WIRE_FLEET_INSTANCE,
            needs_you: [{ ...WIRE_NEEDS_YOU, title: null, project_name: null, step_title: null }],
          },
        ],
      }),
    ]);

    const [instance] = await getFleet();

    expect(instance?.needsYou?.[0]).toMatchObject({ title: null, projectName: null, stepTitle: null });
  });

  it('accepts a needs-you entry whose step_id is null', async () => {
    stubFetch([
      get('/v1/fleet', {
        instances: [{ ...WIRE_FLEET_INSTANCE, needs_you: [{ ...WIRE_NEEDS_YOU, step_id: null }] }],
      }),
    ]);

    const [instance] = await getFleet();

    expect(instance?.needsYou?.[0]?.stepId).toBeNull();
  });

  it('filters a scope this build does not know instead of refusing the answer', async () => {
    stubFetch([
      get('/v1/fleet', {
        instances: [{ ...WIRE_FLEET_INSTANCE, scopes: ['read', 'logs', 'gates'] }],
      }),
    ]);

    const [instance] = await getFleet();

    expect(instance?.scopes).toEqual(['read', 'gates']);
  });

  it.each<[string, unknown]>([
    ['a string present', { ...WIRE_FLEET_INSTANCE, present: 'yes' }],
    ['a string live_runs', { ...WIRE_FLEET_INSTANCE, live_runs: '3' }],
    ['no live_runs', { ...WIRE_FLEET_INSTANCE, live_runs: undefined }],
    ['no label', { ...WIRE_FLEET_INSTANCE, label: undefined }],
    ['a numeric label', { ...WIRE_FLEET_INSTANCE, label: 7 }],
    ['a string last_seen', { ...WIRE_FLEET_INSTANCE, last_seen: '1700000200' }],
    ['scopes that are no list', { ...WIRE_FLEET_INSTANCE, scopes: 'read' }],
    ['a needs_you that is no list', { ...WIRE_FLEET_INSTANCE, needs_you: {} }],
    ['no harnesses', { ...WIRE_FLEET_INSTANCE, harnesses: undefined }],
    ['harnesses that are no list', { ...WIRE_FLEET_INSTANCE, harnesses: 'claude-code' }],
    ['a harness that is no string', { ...WIRE_FLEET_INSTANCE, harnesses: [7] }],
    ['no project_count', { ...WIRE_FLEET_INSTANCE, project_count: undefined }],
    ['a string project_count', { ...WIRE_FLEET_INSTANCE, project_count: '3' }],
    ['a needs-you entry with no title', { ...WIRE_FLEET_INSTANCE, needs_you: [{ ...WIRE_NEEDS_YOU, title: undefined }] }],
    [
      'a needs-you entry with a numeric step_title',
      { ...WIRE_FLEET_INSTANCE, needs_you: [{ ...WIRE_NEEDS_YOU, step_title: 7 }] },
    ],
    [
      'a needs-you entry with no since',
      { ...WIRE_FLEET_INSTANCE, needs_you: [{ ...WIRE_NEEDS_YOU, since: undefined }] },
    ],
    ['null', null],
  ])('refuses an instance with %s', async (_, instance) => {
    stubFetch([get('/v1/fleet', { instances: [instance] })]);

    const error = await rejectionOf(getFleet());

    expect(error.path).toBe('/v1/fleet');
    expect(error.status).toBe(200);
  });

  it('refuses an answer with no instances list', async () => {
    stubFetch([get('/v1/fleet', {})]);

    expect((await rejectionOf(getFleet())).status).toBe(200);
  });
});

describe('getInstance', () => {
  it('maps the detail to its view shape', async () => {
    const { pending } = stubFetch([get(DETAIL_PATH, WIRE_DETAIL)]);

    await expect(getInstance(INSTANCE_ID)).resolves.toEqual({
      id: INSTANCE_ID,
      kind: 'runner',
      present: true,
      connectedAt: 1_700_000_050,
      lastSeen: 1_700_000_200,
      installId: 'install-1',
      os: 'linux',
      arch: 'x86_64',
      version: '1.4.0',
      scopes: ['read', 'spend', 'gates'],
      readGranted: true,
      projects: [{ id: 'p-1', name: 'demeteo', remoteUrl: 'https://example.com/demeteo.git' }],
      liveRuns: [
        {
          instanceId: INSTANCE_ID,
          featureId: 'f-1',
          status: 'running',
          startedAt: 1_700_000_000,
          updatedAt: 1_700_000_300,
          costCents: null,
          title: 'Retry webhooks',
          projectId: 'p-1',
          projectName: 'demeteo',
          stepId: 's-implement',
          stepTitle: 'Implement',
          tickets: { completed: 1, failed: 0, running: 2 },
        },
      ],
      harnesses: ['claude-code'],
      workflows: [
        {
          workflowId: 'wf-feature',
          version: 3,
          name: 'Feature',
          graph: {
            nodes: [
              { id: 's-implement', type: 'agent', title: 'Implement', position: { x: 120, y: 40 } },
              { id: 's-gate', type: 'gate', title: 'Review', position: null },
            ],
            edges: [{ from: 's-implement', to: 's-gate' }],
          },
        },
      ],
      runShape: [
        {
          projectId: 'p-1',
          agentKind: 'claude-code',
          model: 'opus',
          effort: 'xhigh',
          workflowId: 'wf-feature',
          maxBudgetCents: 500,
        },
      ],
    });
    expect(pending).toEqual([]);
  });

  it('keeps every withheld or unknown field null rather than coercing it', async () => {
    stubFetch([
      get(DETAIL_PATH, {
        ...WIRE_DETAIL,
        connected_at: null,
        last_seen: null,
        install_id: null,
        os: null,
        arch: null,
        version: null,
        read_granted: false,
        projects: null,
        live_runs: null,
        harnesses: null,
        workflows: null,
        run_shape: null,
      }),
    ]);

    await expect(getInstance(INSTANCE_ID)).resolves.toMatchObject({
      connectedAt: null,
      lastSeen: null,
      installId: null,
      os: null,
      arch: null,
      version: null,
      readGranted: false,
      projects: null,
      liveRuns: null,
      harnesses: null,
      workflows: null,
      runShape: null,
    });
  });

  it('keeps an empty project, run, workflow and run-shape list empty, distinct from withheld', async () => {
    stubFetch([
      get(DETAIL_PATH, { ...WIRE_DETAIL, projects: [], live_runs: [], workflows: [], run_shape: [] }),
    ]);

    await expect(getInstance(INSTANCE_ID)).resolves.toMatchObject({
      projects: [],
      liveRuns: [],
      workflows: [],
      runShape: [],
    });
  });

  it('accepts a workflow with no readable graph and run-shape defaults that are all null', async () => {
    stubFetch([
      get(DETAIL_PATH, {
        ...WIRE_DETAIL,
        workflows: [{ ...WIRE_WORKFLOW, graph: null }],
        run_shape: [
          {
            project_id: 'p-1',
            default_agent_kind: null,
            default_model: null,
            default_effort: null,
            default_workflow_id: null,
            default_max_budget_cents: null,
          },
        ],
      }),
    ]);

    await expect(getInstance(INSTANCE_ID)).resolves.toMatchObject({
      workflows: [{ workflowId: 'wf-feature', graph: null }],
      runShape: [
        { projectId: 'p-1', agentKind: null, model: null, effort: null, workflowId: null, maxBudgetCents: null },
      ],
    });
  });

  it.each(['extreme', 'High'])('maps the unknown effort %j to null instead of refusing the answer', async (effort) => {
    stubFetch([get(DETAIL_PATH, { ...WIRE_DETAIL, run_shape: [{ ...WIRE_RUN_SHAPE, default_effort: effort }] })]);

    const detail = await getInstance(INSTANCE_ID);

    expect(detail.runShape).toEqual([
      { projectId: 'p-1', agentKind: 'claude-code', model: 'opus', effort: null, workflowId: 'wf-feature', maxBudgetCents: 500 },
    ]);
  });

  it('accepts a run whose started_at and remote_url are null', async () => {
    stubFetch([
      get(DETAIL_PATH, {
        ...WIRE_DETAIL,
        projects: [{ ...WIRE_PROJECT, remote_url: null }],
        live_runs: [{ ...WIRE_RUN, started_at: null }],
      }),
    ]);

    const detail = await getInstance(INSTANCE_ID);

    expect(detail.projects?.[0]?.remoteUrl).toBeNull();
    expect(detail.liveRuns?.[0]?.startedAt).toBeNull();
  });

  it('accepts a numeric cost_cents', async () => {
    stubFetch([get(DETAIL_PATH, { ...WIRE_DETAIL, live_runs: [{ ...WIRE_RUN, cost_cents: 120 }] })]);

    await expect(getInstance(INSTANCE_ID)).resolves.toMatchObject({ liveRuns: [{ costCents: 120 }] });
  });

  it('carries no path a project arrived with, under any key', async () => {
    stubFetch([
      get(DETAIL_PATH, {
        ...WIRE_DETAIL,
        projects: [{ ...WIRE_PROJECT, path: '/home/u/secret', local_path: '/home/u/secret' }],
      }),
    ]);

    const detail = await getInstance(INSTANCE_ID);
    const [project] = detail.projects ?? [];

    expect(Object.keys(project ?? {}).sort()).toEqual(['id', 'name', 'remoteUrl']);
    expect(JSON.stringify(detail)).not.toContain('/home/u/secret');
  });

  it('carries no member of the detail it does not name', async () => {
    stubFetch([get(DETAIL_PATH, { ...WIRE_DETAIL, worktree_root: '/home/u/secret' })]);

    expect(JSON.stringify(await getInstance(INSTANCE_ID))).not.toContain('/home/u/secret');
  });

  it('filters a scope this build does not know instead of refusing the answer', async () => {
    stubFetch([get(DETAIL_PATH, { ...WIRE_DETAIL, scopes: ['logs', 'spend'] })]);

    await expect(getInstance(INSTANCE_ID)).resolves.toMatchObject({ scopes: ['spend'] });
  });

  it.each<[string, unknown]>([
    ['an unknown run status', { ...WIRE_DETAIL, live_runs: [{ ...WIRE_RUN, status: 'paused' }] }],
    [
      'a run status in another case',
      { ...WIRE_DETAIL, live_runs: [{ ...WIRE_RUN, status: 'Running' }] },
    ],
    ['a string present', { ...WIRE_DETAIL, present: 'yes' }],
    ['a string live_runs', { ...WIRE_DETAIL, live_runs: '3' }],
    ['no live_runs', { ...WIRE_DETAIL, live_runs: undefined }],
    ['no projects', { ...WIRE_DETAIL, projects: undefined }],
    ['no install_id', { ...WIRE_DETAIL, install_id: undefined }],
    ['a string cost_cents', { ...WIRE_DETAIL, live_runs: [{ ...WIRE_RUN, cost_cents: '120' }] }],
    ['no cost_cents', { ...WIRE_DETAIL, live_runs: [{ ...WIRE_RUN, cost_cents: undefined }] }],
    ['harnesses that are no list', { ...WIRE_DETAIL, harnesses: 'claude-code' }],
    ['no harnesses', { ...WIRE_DETAIL, harnesses: undefined }],
    ['a run with no tickets', { ...WIRE_DETAIL, live_runs: [{ ...WIRE_RUN, tickets: undefined }] }],
    ['a run with no title', { ...WIRE_DETAIL, live_runs: [{ ...WIRE_RUN, title: undefined }] }],
    ['a project with no name', { ...WIRE_DETAIL, projects: [{ ...WIRE_PROJECT, name: undefined }] }],
    ['a project with no remote_url', { ...WIRE_DETAIL, projects: [{ id: 'p-1', name: 'demeteo' }] }],
    ['no workflows', { ...WIRE_DETAIL, workflows: undefined }],
    ['no run_shape', { ...WIRE_DETAIL, run_shape: undefined }],
    ['workflows that are no list', { ...WIRE_DETAIL, workflows: {} }],
    ['a run_shape that is no list', { ...WIRE_DETAIL, run_shape: 'p-1' }],
    ['a workflow with a string version', { ...WIRE_DETAIL, workflows: [{ ...WIRE_WORKFLOW, version: '3' }] }],
    ['a workflow with no graph', { ...WIRE_DETAIL, workflows: [{ ...WIRE_WORKFLOW, graph: undefined }] }],
    [
      'a graph with no edges',
      { ...WIRE_DETAIL, workflows: [{ ...WIRE_WORKFLOW, graph: { nodes: [WIRE_NODE] } }] },
    ],
    [
      'a node with no type',
      { ...WIRE_DETAIL, workflows: [{ ...WIRE_WORKFLOW, graph: { nodes: [{ ...WIRE_NODE, type: undefined }], edges: [] } }] },
    ],
    [
      'a node with a string position',
      { ...WIRE_DETAIL, workflows: [{ ...WIRE_WORKFLOW, graph: { nodes: [{ ...WIRE_NODE, position: '0,0' }], edges: [] } }] },
    ],
    [
      'a node position with no y',
      { ...WIRE_DETAIL, workflows: [{ ...WIRE_WORKFLOW, graph: { nodes: [{ ...WIRE_NODE, position: { x: 0 } }], edges: [] } }] },
    ],
    [
      'an edge with no to',
      { ...WIRE_DETAIL, workflows: [{ ...WIRE_WORKFLOW, graph: { nodes: [WIRE_NODE], edges: [{ from: 's-implement' }] } }] },
    ],
    ['a run-shape entry with no project_id', { ...WIRE_DETAIL, run_shape: [{ ...WIRE_RUN_SHAPE, project_id: undefined }] }],
    [
      'a run-shape entry with no default_effort',
      { ...WIRE_DETAIL, run_shape: [{ ...WIRE_RUN_SHAPE, default_effort: undefined }] },
    ],
    ['a numeric default_effort', { ...WIRE_DETAIL, run_shape: [{ ...WIRE_RUN_SHAPE, default_effort: 7 }] }],
    [
      'a string default_max_budget_cents',
      { ...WIRE_DETAIL, run_shape: [{ ...WIRE_RUN_SHAPE, default_max_budget_cents: '500' }] },
    ],
    ['null', null],
  ])('refuses a detail with %s', async (_, detail) => {
    stubFetch([get(DETAIL_PATH, detail)]);

    expect((await rejectionOf(getInstance(INSTANCE_ID))).status).toBe(200);
  });

  it('encodes the id into the request path', async () => {
    const upper = INSTANCE_ID.toUpperCase();
    const { sent } = stubFetch([
      get(`/v1/instances/${encodeURIComponent(upper)}`, { ...WIRE_DETAIL, id: upper }),
    ]);

    await getInstance(upper);

    expect(sent).toEqual([`/v1/instances/${encodeURIComponent(upper)}`]);
  });

  it.each(['..', 'a/b', '%2e%2e', '', `${INSTANCE_ID}/..`, `${INSTANCE_ID}?x=1`])(
    'answers %j as not found without sending a request',
    async (instanceId) => {
      const { double } = stubFetch([]);

      const error = await rejectionOf(getInstance(instanceId));

      expect(error.status).toBe(404);
      expect(error.code).toBe('not_found');
      expect(double).not.toHaveBeenCalled();
      expect(unscripted).toEqual([]);
    },
  );

  it('turns a 404 from the Hub into a not_found HubApiError', async () => {
    stubFetch([get(DETAIL_PATH, { error: 'not_found' }, 404)]);

    const error = await rejectionOf(getInstance(INSTANCE_ID));

    expect(error.status).toBe(404);
    expect(error.code).toBe('not_found');
  });
});
