import { afterEach, describe, expect, it, vi } from 'vitest';

import { HubApiError, setCsrfToken } from './api';
import { getRequestStatus, isTerminal, pollRequestStatus, type RequestState, setStepAssignment } from './requests';

type Exchange = { method: 'GET' | 'POST'; path: string; answer: Response };

/** Every request a double was not told about, held to `[]` after each test. */
let unscripted: string[] = [];

/** The scripted double of `runs.test.ts`: a request that is not the next one in `script` throws. */
function stubFetch(script: Exchange[]) {
  const pending = [...script];
  const sent: string[] = [];
  const bodies: unknown[] = [];
  const double = vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
    const method = init?.method ?? 'GET';
    const next = pending.shift();
    if (next === undefined || next.path !== input || next.method !== method) {
      unscripted.push(`${method} ${String(input)}`);
      throw new Error(`fetch double was not told about ${method} ${String(input)} here`);
    }
    sent.push(`${method} ${next.path}`);
    if (typeof init?.body === 'string') bodies.push(JSON.parse(init.body));
    return next.answer;
  });
  vi.stubGlobal('fetch', double);
  return { sent, bodies, pending };
}

function get(path: string, body: unknown, status = 200): Exchange {
  return { method: 'GET', path, answer: json(body, status) };
}

function post(path: string, body: unknown, status = 202): Exchange {
  return { method: 'POST', path, answer: json(body, status) };
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
const REQUEST_ID = 'q7Zr-1_x';
const STATUS_PATH = `/v1/instances/${INSTANCE_ID}/requests/${REQUEST_ID}`;
const ASSIGNMENT_PATH = `/v1/instances/${INSTANCE_ID}/runs/f-1/steps/build/assignment`;

/** Unix seconds; the fake clock starts here. */
const NOW = 1_700_000_000;
const EXPIRES_AT = NOW + 60;
const POLL_TICKET = { requestId: REQUEST_ID, expiresAt: EXPIRES_AT };

/** Well past `EXPIRES_AT`, so a poll that would run on past it gets the chance to. */
const LONG_MS = 10 * 60 * 1000;

function wire(status: Record<string, unknown>, extra: Record<string, unknown> = {}) {
  return {
    request_id: REQUEST_ID,
    kind: 'set_step_assignment',
    ...status,
    deliveries: 1,
    expires_at: EXPIRES_AT,
    answered_at: null,
    ...extra,
  };
}

const COMMON = {
  requestId: REQUEST_ID,
  kind: 'set_step_assignment',
  deliveries: 1,
  expiresAt: EXPIRES_AT,
  answeredAt: null,
};

function useClock() {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
  vi.setSystemTime(NOW * 1000);
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  setCsrfToken(null);
  const strays = unscripted;
  unscripted = [];
  // Soft: a throw here would skip the setup file's own `afterEach`.
  expect.soft(strays).toEqual([]);
});

describe('getRequestStatus', () => {
  it.each([
    [{ status: 'queued' }, { status: 'queued' }],
    [{ status: 'delivered' }, { status: 'delivered' }],
    [{ status: 'accepted' }, { status: 'accepted' }],
    [{ status: 'refused', reason: 'scope_off' }, { status: 'refused', reason: 'scope_off' }],
    [{ status: 'expired' }, { status: 'expired' }],
  ])('maps %o', async (status, expected) => {
    stubFetch([get(STATUS_PATH, wire(status))]);

    await expect(getRequestStatus(INSTANCE_ID, REQUEST_ID)).resolves.toEqual({ ...COMMON, ...expected });
  });

  it.each([
    'credential_not_pinned',
    'some_future_reason',
  ])('keeps the refusal reason %s verbatim', async (reason) => {
    stubFetch([get(STATUS_PATH, wire({ status: 'refused', reason }, { answered_at: NOW - 5 }))]);

    await expect(getRequestStatus(INSTANCE_ID, REQUEST_ID)).resolves.toEqual({
      ...COMMON,
      status: 'refused',
      reason,
      answeredAt: NOW - 5,
    });
  });

  it('refuses a status this build does not know', async () => {
    stubFetch([get(STATUS_PATH, wire({ status: 'pending' }))]);

    const error = await rejectionOf(getRequestStatus(INSTANCE_ID, REQUEST_ID));

    expect(error.message).toMatch(/unexpected shape/);
  });

  it('refuses a refusal that carries no reason', async () => {
    stubFetch([get(STATUS_PATH, wire({ status: 'refused' }))]);

    const error = await rejectionOf(getRequestStatus(INSTANCE_ID, REQUEST_ID));

    expect(error.message).toMatch(/unexpected shape/);
  });

  it('encodes both ids into the request path', async () => {
    const { sent } = stubFetch([get(`/v1/instances/${INSTANCE_ID}/requests/r%2F1%3Fx`, wire({ status: 'queued' }))]);

    await getRequestStatus(INSTANCE_ID, 'r/1?x');

    expect(sent).toEqual([`GET /v1/instances/${INSTANCE_ID}/requests/r%2F1%3Fx`]);
  });

  it('answers an id that is not a UUID as not found, without asking', async () => {
    const { sent } = stubFetch([]);

    const error = await rejectionOf(getRequestStatus('../fleet', REQUEST_ID));

    expect([error.status, error.code]).toEqual([404, 'not_found']);
    expect(sent).toEqual([]);
  });
});

describe('isTerminal', () => {
  it.each([
    [{ status: 'queued' } as const, false],
    [{ status: 'delivered' } as const, false],
    [{ status: 'accepted' } as const, true],
    [{ status: 'refused', reason: 'x' } as const, true],
    [{ status: 'expired' } as const, true],
  ])('%o is terminal: %s', (state, terminal) => {
    expect(isTerminal(state)).toBe(terminal);
  });
});

describe('setStepAssignment', () => {
  const TICKET = { request_id: REQUEST_ID, expires_at: EXPIRES_AT };

  it('posts every provided member and answers the ticket', async () => {
    const { sent, bodies } = stubFetch([post(ASSIGNMENT_PATH, TICKET)]);

    await expect(
      setStepAssignment(INSTANCE_ID, 'f-1', 'build', { agentKind: 'codex', model: 'gpt-5', effort: 'xhigh' }),
    ).resolves.toEqual({ requestId: REQUEST_ID, expiresAt: EXPIRES_AT });
    expect(sent).toEqual([`POST ${ASSIGNMENT_PATH}`]);
    expect(bodies).toEqual([{ agent_kind: 'codex', model: 'gpt-5', effort: 'xhigh' }]);
  });

  it.each([
    [{ effort: 'low' } as const, { effort: 'low' }],
    [{ agentKind: 'pi' }, { agent_kind: 'pi' }],
    [{ model: 'm', effort: undefined }, { model: 'm' }],
  ])('posts only the members of %o', async (assignment, body) => {
    const { bodies } = stubFetch([post(ASSIGNMENT_PATH, TICKET)]);

    await setStepAssignment(INSTANCE_ID, 'f-1', 'build', assignment);

    expect(bodies).toEqual([body]);
    expect(Object.keys(bodies[0] as object)).toEqual(Object.keys(body));
  });

  it('encodes every segment of the path', async () => {
    const path = `/v1/instances/${INSTANCE_ID}/runs/f%2F1/steps/a%3Fb/assignment`;
    const { sent } = stubFetch([post(path, TICKET)]);

    await setStepAssignment(INSTANCE_ID, 'f/1', 'a?b', { effort: 'max' });

    expect(sent).toEqual([`POST ${path}`]);
  });

  it('answers an id that is not a UUID as not found, without asking', async () => {
    const { sent } = stubFetch([]);

    const error = await rejectionOf(setStepAssignment('x', 'f-1', 'build', { effort: 'max' }));

    expect([error.status, error.code]).toEqual([404, 'not_found']);
    expect(sent).toEqual([]);
  });

  it('carries the Hub refusal through', async () => {
    stubFetch([post(ASSIGNMENT_PATH, { error: 'scope_not_granted' }, 403)]);

    const error = await rejectionOf(setStepAssignment(INSTANCE_ID, 'f-1', 'build', { model: 'm' }));

    expect([error.status, error.code]).toEqual([403, 'scope_not_granted']);
  });
});

describe('pollRequestStatus', () => {
  it.each([
    { status: 'accepted' },
    { status: 'refused', reason: 'gate_not_pending' },
    { status: 'expired' },
  ])('reads until %o and no further', async (terminal) => {
    useClock();
    const { sent } = stubFetch([
      get(STATUS_PATH, wire({ status: 'queued' })),
      get(STATUS_PATH, wire({ status: 'delivered' })),
      get(STATUS_PATH, wire(terminal)),
    ]);
    const seen: RequestState[] = [];
    const onStopped = vi.fn();

    pollRequestStatus(INSTANCE_ID, POLL_TICKET, (state) => seen.push(state), { onStopped });
    await vi.advanceTimersByTimeAsync(LONG_MS);

    expect(sent).toHaveLength(3);
    expect(seen.map((state) => state.status)).toEqual(['queued', 'delivered', terminal.status]);
    expect(onStopped).not.toHaveBeenCalled();
  });

  it('reads last at expiresAt, whatever that read says', async () => {
    useClock();
    const live = { status: 'delivered' };
    const { sent, pending } = stubFetch(Array.from({ length: 20 }, () => get(STATUS_PATH, wire(live))));
    const at: number[] = [];

    pollRequestStatus(INSTANCE_ID, POLL_TICKET, () => at.push(Date.now() / 1000));
    await vi.advanceTimersByTimeAsync(LONG_MS);

    expect(at[at.length - 1]).toBe(EXPIRES_AT);
    expect(at.every((second) => second <= EXPIRES_AT)).toBe(true);
    expect(sent).toHaveLength(at.length);
    expect(pending.length).toBeGreaterThan(0);
  });

  it('signals the stop when the read at expiresAt still answers queued, and reads no further', async () => {
    useClock();
    const { sent, pending } = stubFetch(Array.from({ length: 20 }, () => get(STATUS_PATH, wire({ status: 'queued' }))));
    const events: string[] = [];

    pollRequestStatus(INSTANCE_ID, POLL_TICKET, (state) => events.push(`${state.status}@${Date.now() / 1000}`), {
      onStopped: () => events.push(`stopped@${Date.now() / 1000}`),
    });
    await vi.advanceTimersByTimeAsync(LONG_MS);

    expect(events.slice(-2)).toEqual([`queued@${EXPIRES_AT}`, `stopped@${EXPIRES_AT}`]);
    expect(events.filter((event) => event.startsWith('stopped'))).toHaveLength(1);
    expect(sent).toHaveLength(events.length - 1);
    expect(pending.length).toBeGreaterThan(0);
  });

  it('reads nothing after the ticket expiry when no read ever succeeds', async () => {
    useClock();
    const { sent, pending } = stubFetch(
      Array.from({ length: 20 }, () => get(STATUS_PATH, { error: 'scope_not_granted' }, 409)),
    );
    const at: number[] = [];
    const onStopped = vi.fn();

    pollRequestStatus(INSTANCE_ID, POLL_TICKET, () => {}, { onError: () => at.push(Date.now() / 1000), onStopped });
    await vi.advanceTimersByTimeAsync(LONG_MS);

    expect(at[at.length - 1]).toBe(EXPIRES_AT);
    expect(at.every((second) => second <= EXPIRES_AT)).toBe(true);
    expect(sent).toHaveLength(at.length);
    expect(pending.length).toBeGreaterThan(0);
    expect(onStopped).toHaveBeenCalledTimes(1);
  });

  it('reads once and stops when the ticket has already expired', async () => {
    useClock();
    const { sent } = stubFetch([get(STATUS_PATH, wire({ status: 'accepted' }, { answered_at: NOW - 70 }))]);
    const seen: string[] = [];

    pollRequestStatus(INSTANCE_ID, { requestId: REQUEST_ID, expiresAt: NOW - 1 }, (state) => seen.push(state.status));
    await vi.advanceTimersByTimeAsync(LONG_MS);

    expect(sent).toHaveLength(1);
    expect(seen).toEqual(['accepted']);
  });

  it('stops on a 401 and reports it', async () => {
    useClock();
    const { sent } = stubFetch([
      get(STATUS_PATH, wire({ status: 'queued' })),
      get(STATUS_PATH, { error: 'unauthenticated' }, 401),
    ]);
    const errors: unknown[] = [];

    pollRequestStatus(INSTANCE_ID, POLL_TICKET, () => {}, { onError: (error) => errors.push(error) });
    await vi.advanceTimersByTimeAsync(LONG_MS);

    expect(sent).toHaveLength(2);
    expect(errors).toEqual([expect.objectContaining({ status: 401 })]);
  });

  it('reads again after a failure that is not a 401', async () => {
    useClock();
    const { sent } = stubFetch([
      get(STATUS_PATH, { error: 'store_failed' }, 500),
      get(STATUS_PATH, wire({ status: 'accepted' })),
    ]);
    const seen: string[] = [];
    const errors: unknown[] = [];

    pollRequestStatus(INSTANCE_ID, POLL_TICKET, (state) => seen.push(state.status), {
      onError: (error) => errors.push(error),
    });
    await vi.advanceTimersByTimeAsync(LONG_MS);

    expect(sent).toHaveLength(2);
    expect(seen).toEqual(['accepted']);
    expect(errors).toEqual([expect.objectContaining({ status: 500 })]);
  });

  it('reads nothing more once cancelled between reads', async () => {
    useClock();
    const { sent } = stubFetch([get(STATUS_PATH, wire({ status: 'queued' }))]);
    const seen: string[] = [];

    const cancel = pollRequestStatus(INSTANCE_ID, POLL_TICKET, (state) => seen.push(state.status));
    await vi.advanceTimersByTimeAsync(0);
    cancel();
    await vi.advanceTimersByTimeAsync(LONG_MS);

    expect(sent).toHaveLength(1);
    expect(seen).toEqual(['queued']);
  });

  it('drops a read that settles after cancel, and reads nothing more', async () => {
    useClock();
    const { sent } = stubFetch([get(STATUS_PATH, wire({ status: 'queued' }))]);
    const seen: string[] = [];

    const cancel = pollRequestStatus(INSTANCE_ID, POLL_TICKET, (state) => seen.push(state.status));
    cancel();
    await vi.advanceTimersByTimeAsync(LONG_MS);

    expect(sent).toHaveLength(1);
    expect(seen).toEqual([]);
  });
});
