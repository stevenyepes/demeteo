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
};

const WIRE_FLEET_INSTANCE = {
  id: INSTANCE_ID,
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
};

const WIRE_RUN = {
  instance_id: INSTANCE_ID,
  feature_id: 'f-1',
  status: 'running',
  started_at: 1_700_000_000,
  updated_at: 1_700_000_300,
  cost_cents: null,
};

const WIRE_PROJECT = { id: 'p-1', name: 'demeteo', remote_url: 'https://example.com/demeteo.git' };

const WIRE_DETAIL = {
  id: INSTANCE_ID,
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
  harnesses: null,
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
  it('maps every wire instance to its view shape, in the order sent', async () => {
    const { pending } = stubFetch([
      get('/v1/fleet', {
        instances: [
          WIRE_FLEET_INSTANCE,
          {
            ...WIRE_FLEET_INSTANCE,
            id: OTHER_ID,
            label: 'desk',
            present: false,
            live_runs: 0,
            needs_you: [],
          },
        ],
      }),
    ]);

    await expect(getFleet()).resolves.toEqual([
      {
        id: INSTANCE_ID,
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
          },
        ],
      },
      {
        id: OTHER_ID,
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
          },
        ],
      }),
    ]);

    const [instance] = await getFleet();

    expect(instance).toMatchObject({
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
        },
      ],
      harnesses: null,
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
    });
  });

  it('keeps an empty project and run list empty, distinct from withheld', async () => {
    stubFetch([get(DETAIL_PATH, { ...WIRE_DETAIL, projects: [], live_runs: [] })]);

    await expect(getInstance(INSTANCE_ID)).resolves.toMatchObject({ projects: [], liveRuns: [] });
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
    ['harnesses that are not null', { ...WIRE_DETAIL, harnesses: ['claude-code'] }],
    ['no harnesses', { ...WIRE_DETAIL, harnesses: undefined }],
    ['a project with no name', { ...WIRE_DETAIL, projects: [{ ...WIRE_PROJECT, name: undefined }] }],
    ['a project with no remote_url', { ...WIRE_DETAIL, projects: [{ id: 'p-1', name: 'demeteo' }] }],
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
