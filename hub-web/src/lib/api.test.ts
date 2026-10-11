import { afterEach, describe, expect, it, vi } from 'vitest';

import { getJson, HubApiError } from './api';

interface Fleet {
  instances: string[];
}

function isFleet(value: unknown): value is Fleet {
  if (typeof value !== 'object' || value === null) return false;
  const instances: unknown = Reflect.get(value, 'instances');
  return Array.isArray(instances) && instances.every((id) => typeof id === 'string');
}

/**
 * Answers only the URLs in `answers` and throws on every other one, so a test
 * that reaches `fetch` with a URL it did not name fails on that instead of
 * being asserted against a default reply. An `Error` answer is a rejection.
 */
function stubFetch(answers: Record<string, Response | Error> = {}) {
  const double = vi.fn(async (input: unknown): Promise<Response> => {
    const answer = typeof input === 'string' ? answers[input] : undefined;
    if (answer === undefined) throw new Error(`fetch double was not told about ${String(input)}`);
    if (answer instanceof Error) throw answer;
    return answer;
  });
  vi.stubGlobal('fetch', double);
  return double;
}

function json(body: unknown, status = 200): Response {
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
  if (!(error instanceof HubApiError)) throw new Error(`expected a HubApiError, got ${String(error)}`);
  return error;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('getJson', () => {
  it('resolves to a 2xx JSON body the guard accepts', async () => {
    const fetchDouble = stubFetch({ '/fleet': json({ instances: ['i-1', 'i-2'] }) });

    await expect(getJson('/fleet', isFleet)).resolves.toEqual({ instances: ['i-1', 'i-2'] });
    expect(fetchDouble).toHaveBeenCalledTimes(1);
  });

  it('rejects a 2xx body the guard refuses', async () => {
    stubFetch({ '/fleet': json({ instances: 'i-1' }) });

    const error = await rejectionOf(getJson('/fleet', isFleet));

    expect(error.path).toBe('/fleet');
    expect(error.status).toBe(200);
  });

  it('rejects a 2xx body that is not JSON', async () => {
    stubFetch({ '/fleet': new Response('<!doctype html>', { status: 200 }) });

    const error = await rejectionOf(getJson('/fleet', isFleet));

    expect(error.status).toBe(200);
  });

  it.each([404, 500])('rejects a %i response, carrying the status', async (status) => {
    stubFetch({ '/fleet': json({ instances: [] }, status) });

    const error = await rejectionOf(getJson('/fleet', isFleet));

    expect(error.path).toBe('/fleet');
    expect(error.status).toBe(status);
  });

  it('rejects a failed request with a null status', async () => {
    stubFetch({ '/fleet': new TypeError('connection refused') });

    const error = await rejectionOf(getJson('/fleet', isFleet));

    expect(error.status).toBeNull();
    expect(error.path).toBe('/fleet');
    // The double's own refusal is a rejection too; the cause tells them apart.
    expect(error.message).toContain('connection refused');
  });

  it.each([
    'https://evil.example/x',
    '//evil.example/x',
    '/redirect?to=https://evil.example/x',
    'fleet',
    '',
    '/\\evil.example/x',
    '/\t/evil.example/x',
    '/\n/evil.example/x',
  ])('rejects the path %j without calling fetch', async (path) => {
    const fetchDouble = stubFetch();

    const error = await rejectionOf(getJson(path, isFleet));

    expect(error.path).toBe(path);
    expect(error.status).toBeNull();
    expect(fetchDouble).not.toHaveBeenCalled();
  });
});
