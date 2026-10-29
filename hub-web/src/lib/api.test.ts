import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  dropSession,
  getJson,
  HubApiError,
  onSessionGone,
  postBytes,
  postJson,
  setCsrfToken,
} from './api';

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
  const double = vi.fn(async (input: unknown, _init?: RequestInit): Promise<Response> => {
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

function sentHeaders(fetchDouble: ReturnType<typeof stubFetch>): Headers {
  return new Headers(fetchDouble.mock.calls[0]?.[1]?.headers);
}

afterEach(() => {
  vi.unstubAllGlobals();
  setCsrfToken(null);
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
    expect(error.code).toBeNull();
  });

  it('carries the error code of a non-2xx JSON body', async () => {
    stubFetch({ '/v1/session': json({ error: 'unauthenticated' }, 401) });

    const error = await rejectionOf(getJson('/v1/session', isFleet));

    expect(error.status).toBe(401);
    expect(error.code).toBe('unauthenticated');
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

describe('postJson', () => {
  it('posts the body as JSON and resolves to a 2xx body the guard accepts', async () => {
    const fetchDouble = stubFetch({ '/v1/fleet': json({ instances: ['i-1'] }) });

    await expect(postJson('/v1/fleet', { label: 'laptop' }, isFleet)).resolves.toEqual({
      instances: ['i-1'],
    });

    expect(fetchDouble).toHaveBeenCalledTimes(1);
    const init = fetchDouble.mock.calls[0]?.[1];
    expect(init?.method).toBe('POST');
    expect(init?.body).toBe('{"label":"laptop"}');
    expect(sentHeaders(fetchDouble).get('Content-Type')).toBe('application/json');
  });

  it('sends X-CSRF-Token only while a token is set', async () => {
    const before = stubFetch({ '/v1/fleet': json({ instances: [] }) });
    await postJson('/v1/fleet', {}, isFleet);
    expect(sentHeaders(before).has('X-CSRF-Token')).toBe(false);

    setCsrfToken('tok-1');
    const during = stubFetch({ '/v1/fleet': json({ instances: [] }) });
    await postJson('/v1/fleet', {}, isFleet);
    expect(sentHeaders(during).get('X-CSRF-Token')).toBe('tok-1');

    setCsrfToken(null);
    const after = stubFetch({ '/v1/fleet': json({ instances: [] }) });
    await postJson('/v1/fleet', {}, isFleet);
    expect(sentHeaders(after).has('X-CSRF-Token')).toBe(false);
  });

  it('keeps the CSRF token out of every browser store', () => {
    setCsrfToken('tok-1');

    expect(localStorage.length).toBe(0);
    expect(sessionStorage.length).toBe(0);
    expect(document.cookie).toBe('');
  });

  it('rejects a non-2xx response, carrying its status and error code', async () => {
    stubFetch({ '/v1/fleet': json({ error: 'self_endorsement' }, 403) });

    const error = await rejectionOf(postJson('/v1/fleet', {}, isFleet));

    expect(error.path).toBe('/v1/fleet');
    expect(error.status).toBe(403);
    expect(error.code).toBe('self_endorsement');
  });

  it.each([
    ['no body', new Response(null, { status: 502 })],
    ['a body that is not JSON', new Response('<!doctype html>', { status: 502 })],
    ['a JSON body with no error string', json({ error: 7 }, 502)],
  ])('rejects a non-2xx response with %s, with a null code', async (_shape, response) => {
    stubFetch({ '/v1/fleet': response });

    const error = await rejectionOf(postJson('/v1/fleet', {}, isFleet));

    expect(error.status).toBe(502);
    expect(error.code).toBeNull();
  });

  it('rejects a 2xx body the guard refuses', async () => {
    stubFetch({ '/v1/fleet': json({ instances: 'i-1' }) });

    const error = await rejectionOf(postJson('/v1/fleet', {}, isFleet));

    expect(error.status).toBe(200);
    expect(error.code).toBeNull();
  });

  it.each(['https://evil.example/x', '//evil.example/x', 'v1/fleet', '/\\evil.example/x'])(
    'rejects the path %j without calling fetch',
    async (path) => {
      const fetchDouble = stubFetch();

      const error = await rejectionOf(postJson(path, {}, isFleet));

      expect(error.path).toBe(path);
      expect(error.status).toBeNull();
      expect(fetchDouble).not.toHaveBeenCalled();
    },
  );
});

describe('postBytes', () => {
  it('posts the Blob itself as the body, with no Content-Type of its own', async () => {
    setCsrfToken('tok-1');
    const fetchDouble = stubFetch({ '/v1/instances/i-1/attachments': json({ instances: ['a-1'] }, 201) });
    const file = new Blob(['%PDF-1.7 raw'], { type: 'application/pdf' });

    await expect(postBytes('/v1/instances/i-1/attachments', file, isFleet)).resolves.toEqual({
      instances: ['a-1'],
    });

    expect(fetchDouble).toHaveBeenCalledTimes(1);
    const init = fetchDouble.mock.calls[0]?.[1];
    expect(init?.method).toBe('POST');
    expect(init?.body).toBe(file);
    expect(sentHeaders(fetchDouble).get('X-CSRF-Token')).toBe('tok-1');
    expect(sentHeaders(fetchDouble).has('Content-Type')).toBe(false);
  });

  it('sends no X-CSRF-Token while no token is set', async () => {
    const before = stubFetch({ '/v1/up': json({ instances: [] }) });
    await postBytes('/v1/up', new Blob(['x']), isFleet);
    expect(sentHeaders(before).has('X-CSRF-Token')).toBe(false);
  });

  it('rejects a 413 with the code attachment_too_large', async () => {
    setCsrfToken('tok-1');
    stubFetch({ '/v1/up': json({ error: 'attachment_too_large' }, 413) });

    const error = await rejectionOf(postBytes('/v1/up', new Blob(['x']), isFleet));

    expect(error.status).toBe(413);
    expect(error.code).toBe('attachment_too_large');
    expect(error.scope).toBeNull();
  });

  it.each(['https://evil.example/x', '//evil.example/x', 'v1/up', '/\\evil.example/x'])(
    'rejects the path %j without calling fetch',
    async (path) => {
      const fetchDouble = stubFetch();

      const error = await rejectionOf(postBytes(path, new Blob(['x']), isFleet));

      expect(error.status).toBeNull();
      expect(fetchDouble).not.toHaveBeenCalled();
    },
  );
});

describe('HubApiError.scope', () => {
  it('carries the scope of a scope_not_granted answer', async () => {
    stubFetch({ '/v1/up': json({ error: 'scope_not_granted', scope: 'spend' }, 409) });

    const error = await rejectionOf(postBytes('/v1/up', new Blob(['x']), isFleet));

    expect(error.status).toBe(409);
    expect(error.code).toBe('scope_not_granted');
    expect(error.scope).toBe('spend');
  });

  it.each([
    ['another error code', json({ error: 'forbidden' }, 403)],
    ['a scope that is not a string', json({ error: 'scope_not_granted', scope: 7 }, 409)],
    ['a body that is not JSON', new Response('<!doctype html>', { status: 409 })],
  ])('is null for %s', async (_shape, response) => {
    stubFetch({ '/v1/fleet': response });

    const error = await rejectionOf(postJson('/v1/fleet', {}, isFleet));

    expect(error.scope).toBeNull();
  });
});

describe('a 401 answered while a token is held', () => {
  const UNAUTHENTICATED = { error: 'unauthenticated' };

  /** Unsubscribed by the test that made it: a listener left behind would be told about the next test's 401. */
  function listen() {
    const told = vi.fn();
    return { told, stop: onSessionGone(told) };
  }

  it.each([
    ['getJson', () => getJson('/v1/passkeys', isFleet)],
    ['postJson', () => postJson('/v1/passkeys', {}, isFleet)],
    ['postBytes', () => postBytes('/v1/passkeys', new Blob(['x']), isFleet)],
  ])('through %s, tells the listener once, drops the token and still rejects', async (_verb, send) => {
    const { told, stop } = listen();
    setCsrfToken('tok-1');
    stubFetch({ '/v1/passkeys': json(UNAUTHENTICATED, 401) });

    const error = await rejectionOf(send());

    expect(error.status).toBe(401);
    expect(error.code).toBe('unauthenticated');
    expect(told).toHaveBeenCalledTimes(1);
    const after = stubFetch({ '/v1/fleet': json({ instances: [] }) });
    await postJson('/v1/fleet', {}, isFleet);
    expect(sentHeaders(after).has('X-CSRF-Token')).toBe(false);
    stop();
  });

  it('tells nobody when no token is held', async () => {
    const { told, stop } = listen();
    stubFetch({ '/v1/session': json(UNAUTHENTICATED, 401) });

    await rejectionOf(getJson('/v1/session', isFleet));
    await rejectionOf(postJson('/v1/session', {}, isFleet));

    expect(told).not.toHaveBeenCalled();
    stop();
  });

  it.each([403, 500])('tells nobody and keeps the token on a %i', async (status) => {
    const { told, stop } = listen();
    setCsrfToken('tok-1');
    stubFetch({ '/v1/passkeys': json({ error: 'internal' }, status) });

    await rejectionOf(getJson('/v1/passkeys', isFleet));

    expect(told).not.toHaveBeenCalled();
    const after = stubFetch({ '/v1/fleet': json({ instances: [] }) });
    await postJson('/v1/fleet', {}, isFleet);
    expect(sentHeaders(after).get('X-CSRF-Token')).toBe('tok-1');
    stop();
  });

  it('tells the listener once when two requests of one session are both answered 401', async () => {
    const { told, stop } = listen();
    setCsrfToken('tok-1');
    stubFetch({ '/v1/passkeys': json(UNAUTHENTICATED, 401), '/v1/fleet': json(UNAUTHENTICATED, 401) });

    await Promise.all([
      rejectionOf(getJson('/v1/passkeys', isFleet)),
      rejectionOf(postJson('/v1/fleet', {}, isFleet)),
    ]);

    expect(told).toHaveBeenCalledTimes(1);
    stop();
  });

  it('leaves alone the token of a sign-in that finished while the request was in flight', async () => {
    const { told, stop } = listen();
    setCsrfToken('tok-1');
    let answerLate: (response: Response) => void = () => {};
    vi.stubGlobal(
      'fetch',
      vi.fn(() => new Promise<Response>((resolve) => {
        answerLate = resolve;
      })),
    );
    const inFlight = rejectionOf(getJson('/v1/passkeys', isFleet));

    setCsrfToken('tok-2');
    answerLate(json(UNAUTHENTICATED, 401));

    expect((await inFlight).status).toBe(401);
    expect(told).not.toHaveBeenCalled();
    const after = stubFetch({ '/v1/fleet': json({ instances: [] }) });
    await postJson('/v1/fleet', {}, isFleet);
    expect(sentHeaders(after).get('X-CSRF-Token')).toBe('tok-2');
    stop();
  });

  it('stops telling a listener that unsubscribed', async () => {
    const first = listen();
    const second = listen();
    first.stop();
    setCsrfToken('tok-1');
    stubFetch({ '/v1/passkeys': json(UNAUTHENTICATED, 401) });

    await rejectionOf(getJson('/v1/passkeys', isFleet));

    expect(first.told).not.toHaveBeenCalled();
    expect(second.told).toHaveBeenCalledTimes(1);
    second.stop();
  });
});

describe('dropSession', () => {
  it('drops the token and tells every listener', async () => {
    const told = vi.fn();
    const stop = onSessionGone(told);
    setCsrfToken('tok-1');

    dropSession();

    expect(told).toHaveBeenCalledTimes(1);
    const after = stubFetch({ '/v1/fleet': json({ instances: [] }) });
    await postJson('/v1/fleet', {}, isFleet);
    expect(sentHeaders(after).has('X-CSRF-Token')).toBe(false);
    stop();
  });
});
