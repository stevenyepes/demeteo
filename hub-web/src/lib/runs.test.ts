import { afterEach, describe, expect, it, vi } from 'vitest';

import { HubApiError, setCsrfToken } from './api';
import { getRunDetail, listRuns } from './runs';

type Exchange = { method: 'GET'; path: string; answer: Response };

/** Every request a double was not told about, held to `[]` after each test. */
let unscripted: string[] = [];

/** Every path any double was asked for, checked against the module's two routes after each test. */
let asked: string[] = [];

/** The scripted double of `fleet.test.ts`: a request that is not the next one in `script` throws. */
function stubFetch(script: Exchange[]) {
  const pending = [...script];
  const sent: string[] = [];
  const double = vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
    const method = init?.method ?? 'GET';
    asked.push(String(input));
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

/** The only routes this module may name: no transcript, diff or live-output path exists here. */
const ALLOWED = [
  /^\/v1\/runs(\?[^/]*)?$/,
  /^\/v1\/instances\/[^/?]+\/runs\/[^/?]+$/,
];

const WIRE_RUN = {
  instance_id: INSTANCE_ID,
  feature_id: 'f-1',
  status: 'running',
  started_at: 1_700_000_000,
  updated_at: 1_700_000_300,
  cost_cents: null,
};

const WORKFLOW = {
  schema_version: 2,
  id: 'wf-1',
  name: 'Feature',
  nodes: [
    { id: 'plan', type: 'agent' },
    { id: 'review', type: 'gate' },
  ],
  edges: [{ from: 'plan', to: 'review' }],
};

const WIRE_PENDING_GATE = {
  feature_id: 'f-1',
  step_execution_id: 'se-2',
  step_id: 'review',
  since: 1_700_000_250,
};

const WIRE_EVENT = {
  offset: 4,
  kind: 'step_started',
  occurred_at: 1_700_000_100,
  step_id: 'plan',
  step_execution_id: 'se-1',
  ticket_id: null,
};

const WIRE_DETAIL = {
  instance_id: INSTANCE_ID,
  feature_id: 'f-1',
  status: 'running',
  workflow: null,
  steps: [
    { step_id: 'plan', state: 'completed' },
    { step_id: 'build', state: 'failed' },
  ],
  tickets: { completed: 2, failed: 1, running: 3 },
  pending_gates: [WIRE_PENDING_GATE],
  events: [WIRE_EVENT],
};

const DETAIL_PATH = `/v1/instances/${INSTANCE_ID}/runs/f-1`;

afterEach(() => {
  vi.unstubAllGlobals();
  setCsrfToken(null);
  const strays = unscripted;
  const paths = asked;
  unscripted = [];
  asked = [];
  // Soft: a throw here would skip the setup file's own `afterEach`.
  expect.soft(strays).toEqual([]);
  expect.soft(paths.filter((path) => !ALLOWED.some((route) => route.test(path)))).toEqual([]);
});

describe('listRuns', () => {
  it('maps every wire run to its view shape, in the order sent', async () => {
    const { pending } = stubFetch([
      get('/v1/runs', {
        runs: [
          WIRE_RUN,
          { ...WIRE_RUN, instance_id: OTHER_ID, feature_id: 'f-2', status: 'cancelled', started_at: null },
        ],
      }),
    ]);

    await expect(listRuns()).resolves.toEqual([
      {
        instanceId: INSTANCE_ID,
        featureId: 'f-1',
        status: 'running',
        startedAt: 1_700_000_000,
        updatedAt: 1_700_000_300,
        costCents: null,
      },
      {
        instanceId: OTHER_ID,
        featureId: 'f-2',
        status: 'cancelled',
        startedAt: null,
        updatedAt: 1_700_000_300,
        costCents: null,
      },
    ]);
    expect(pending).toEqual([]);
  });

  it('keeps an empty list empty', async () => {
    stubFetch([get('/v1/runs', { runs: [] })]);

    await expect(listRuns()).resolves.toEqual([]);
  });

  it('sends the instance and status filter as an encoded query', async () => {
    const { sent } = stubFetch([
      get(`/v1/runs?instance=${INSTANCE_ID}&status=failed`, { runs: [] }),
      get('/v1/runs?instance=a%2Fb%3Fc%26d', { runs: [] }),
    ]);

    await listRuns({ instance: INSTANCE_ID, status: 'failed' });
    await listRuns({ instance: 'a/b?c&d' });

    expect(sent).toEqual([`/v1/runs?instance=${INSTANCE_ID}&status=failed`, '/v1/runs?instance=a%2Fb%3Fc%26d']);
  });

  it.each<[string, unknown]>([
    ['an unknown status', { ...WIRE_RUN, status: 'paused' }],
    ['a status in another case', { ...WIRE_RUN, status: 'Running' }],
    ['no feature_id', { ...WIRE_RUN, feature_id: undefined }],
    ['no updated_at', { ...WIRE_RUN, updated_at: undefined }],
    ['a string cost_cents', { ...WIRE_RUN, cost_cents: '120' }],
    ['null', null],
  ])('refuses a run with %s', async (_, run) => {
    stubFetch([get('/v1/runs', { runs: [run] })]);

    const error = await rejectionOf(listRuns());

    expect(error.path).toBe('/v1/runs');
    expect(error.status).toBe(200);
  });

  it('refuses an answer with no runs list', async () => {
    stubFetch([get('/v1/runs', {})]);

    expect((await rejectionOf(listRuns())).status).toBe(200);
  });
});

describe('getRunDetail', () => {
  it('maps the detail to its view shape', async () => {
    const { pending } = stubFetch([get(DETAIL_PATH, WIRE_DETAIL)]);

    await expect(getRunDetail(INSTANCE_ID, 'f-1')).resolves.toEqual({
      instanceId: INSTANCE_ID,
      featureId: 'f-1',
      status: 'running',
      workflow: null,
      steps: [
        { stepId: 'plan', state: 'completed' },
        { stepId: 'build', state: 'failed' },
      ],
      tickets: { completed: 2, failed: 1, running: 3 },
      pendingGates: [{ featureId: 'f-1', stepExecutionId: 'se-2', stepId: 'review', since: 1_700_000_250 }],
      events: [
        {
          offset: 4,
          kind: 'step_started',
          occurredAt: 1_700_000_100,
          stepId: 'plan',
          stepExecutionId: 'se-1',
          ticketId: null,
        },
      ],
    });
    expect(pending).toEqual([]);
  });

  it('carries a reported workflow through as the definition the canvas draws', async () => {
    stubFetch([get(DETAIL_PATH, { ...WIRE_DETAIL, workflow: WORKFLOW })]);

    await expect(getRunDetail(INSTANCE_ID, 'f-1')).resolves.toMatchObject({ workflow: WORKFLOW });
  });

  it('accepts an event kind this build does not know, as sent', async () => {
    stubFetch([get(DETAIL_PATH, { ...WIRE_DETAIL, events: [{ ...WIRE_EVENT, kind: 'some_future_kind' }] })]);

    const detail = await getRunDetail(INSTANCE_ID, 'f-1');

    expect(detail.events[0]?.kind).toBe('some_future_kind');
  });

  it('accepts a pending gate whose step_id is null', async () => {
    stubFetch([get(DETAIL_PATH, { ...WIRE_DETAIL, pending_gates: [{ ...WIRE_PENDING_GATE, step_id: null }] })]);

    const detail = await getRunDetail(INSTANCE_ID, 'f-1');

    expect(detail.pendingGates[0]?.stepId).toBeNull();
  });

  it('carries no member of the detail it does not name', async () => {
    stubFetch([
      get(DETAIL_PATH, {
        ...WIRE_DETAIL,
        transcript: 'secret transcript',
        tickets: { ...WIRE_DETAIL.tickets, diff: 'secret diff' },
      }),
    ]);

    const detail = JSON.stringify(await getRunDetail(INSTANCE_ID, 'f-1'));

    expect(detail).not.toContain('secret transcript');
    expect(detail).not.toContain('secret diff');
  });

  it.each<[string, unknown]>([
    ['an unknown status', { ...WIRE_DETAIL, status: 'paused' }],
    ['an unknown step state', { ...WIRE_DETAIL, steps: [{ step_id: 'plan', state: 'awaiting_gate' }] }],
    ['a step with no step_id', { ...WIRE_DETAIL, steps: [{ state: 'running' }] }],
    ['no workflow', { ...WIRE_DETAIL, workflow: undefined }],
    ['a workflow with no nodes', { ...WIRE_DETAIL, workflow: { ...WORKFLOW, nodes: undefined } }],
    ['a workflow node with no id', { ...WIRE_DETAIL, workflow: { ...WORKFLOW, nodes: [{ type: 'agent' }] } }],
    ['a workflow with edges that are no list', { ...WIRE_DETAIL, workflow: { ...WORKFLOW, edges: {} } }],
    ['a workflow edge with no end', { ...WIRE_DETAIL, workflow: { ...WORKFLOW, edges: [{ from: 'plan' }] } }],
    ['a workflow that is a string', { ...WIRE_DETAIL, workflow: 'plan -> review' }],
    ['no steps', { ...WIRE_DETAIL, steps: undefined }],
    ['no tickets', { ...WIRE_DETAIL, tickets: undefined }],
    ['a string ticket count', { ...WIRE_DETAIL, tickets: { completed: '2', failed: 1, running: 3 } }],
    ['no pending_gates', { ...WIRE_DETAIL, pending_gates: undefined }],
    [
      'a pending gate with no step_execution_id',
      { ...WIRE_DETAIL, pending_gates: [{ ...WIRE_PENDING_GATE, step_execution_id: undefined }] },
    ],
    ['no events', { ...WIRE_DETAIL, events: undefined }],
    ['an event with no kind', { ...WIRE_DETAIL, events: [{ ...WIRE_EVENT, kind: undefined }] }],
    ['an event with no ticket_id', { ...WIRE_DETAIL, events: [{ ...WIRE_EVENT, ticket_id: undefined }] }],
    ['an event with a numeric kind', { ...WIRE_DETAIL, events: [{ ...WIRE_EVENT, kind: 7 }] }],
    ['null', null],
  ])('refuses a detail with %s', async (_, detail) => {
    stubFetch([get(DETAIL_PATH, detail)]);

    const error = await rejectionOf(getRunDetail(INSTANCE_ID, 'f-1'));

    expect(error.path).toBe(DETAIL_PATH);
    expect(error.status).toBe(200);
  });

  it('encodes both ids into the request path', async () => {
    const upper = INSTANCE_ID.toUpperCase();
    const path = `/v1/instances/${encodeURIComponent(upper)}/runs/${encodeURIComponent('f/1?x#y')}`;
    const { sent } = stubFetch([get(path, { ...WIRE_DETAIL, instance_id: upper, feature_id: 'f/1?x#y' })]);

    await getRunDetail(upper, 'f/1?x#y');

    expect(sent).toEqual([`/v1/instances/${upper}/runs/f%2F1%3Fx%23y`]);
  });

  it.each(['..', 'a/b', '%2e%2e', '', `${INSTANCE_ID}/..`, `${INSTANCE_ID}?x=1`])(
    'answers instance %j as not found without sending a request',
    async (instanceId) => {
      const { double } = stubFetch([]);

      const error = await rejectionOf(getRunDetail(instanceId, 'f-1'));

      expect(error.status).toBe(404);
      expect(error.code).toBe('not_found');
      expect(double).not.toHaveBeenCalled();
    },
  );

  it('turns a 404 from the Hub into a not_found HubApiError', async () => {
    stubFetch([get(DETAIL_PATH, { error: 'not_found' }, 404)]);

    const error = await rejectionOf(getRunDetail(INSTANCE_ID, 'f-1'));

    expect(error.status).toBe(404);
    expect(error.code).toBe('not_found');
  });
});
