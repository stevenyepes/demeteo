import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { dropSession, type Guard, HubApiError, onSessionGone, setCsrfToken } from './api';
import { fetchSession, postInSession, sessionStillHeld, signIn, signOut, signUp } from './auth';

const webauthn = vi.hoisted(() => ({
  createCredential: vi.fn<(options: unknown) => Promise<unknown>>(),
  getCredential: vi.fn<(options: unknown) => Promise<unknown>>(),
}));

vi.mock('./webauthn', () => webauthn);

type Exchange = { method: 'GET' | 'POST'; path: string; answer: Response };

type Sent = { path: string; body: unknown; csrfToken: string | null };

/**
 * Answers `script` in order and throws on any request that is not the next
 * one in it, so a flow that skips, repeats or reorders a request fails on that
 * instead of being asserted against a default reply.
 */
function stubFetch(script: Exchange[]) {
  const pending = [...script];
  const sent: Sent[] = [];
  const double = vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
    const method = init?.method ?? 'GET';
    const next = pending.shift();
    if (next === undefined || next.path !== input || next.method !== method) {
      throw new Error(`fetch double was not told about ${method} ${String(input)} here`);
    }
    sent.push({
      path: next.path,
      body: typeof init?.body === 'string' ? (JSON.parse(init.body) as unknown) : undefined,
      csrfToken: new Headers(init?.headers).get('X-CSRF-Token'),
    });
    return next.answer;
  });
  vi.stubGlobal('fetch', double);
  return { sent, pending };
}

function post(path: string, body: unknown, status = 200): Exchange {
  return { method: 'POST', path, answer: json(body, status) };
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
  if (!(error instanceof HubApiError)) throw new Error(`expected a HubApiError, got ${String(error)}`);
  return error;
}

function refuse(double: typeof webauthn.createCredential, name: string): void {
  double.mockImplementation(async () => {
    throw new Error(`${name} double was not told to answer`);
  });
}

const CREATION_OPTIONS = { publicKey: { challenge: 'Y3JlYXRl' } };
const REQUEST_OPTIONS = { publicKey: { challenge: 'Z2V0' } };
const ATTESTATION = { id: 'cred-1', response: { attestationObject: 'YXR0' } };
const ASSERTION = { id: 'cred-1', response: { signature: 'c2ln' } };
const WIRE_SESSION = { csrf_token: 'tok-1', credential_id: 'cred-1' };
const SESSION = { csrfToken: 'tok-1', credentialId: 'cred-1' };

const WRONG_SESSIONS: [string, unknown][] = [
  ['no csrf_token', { credential_id: 'cred-1' }],
  ['no credential_id', { csrf_token: 'tok-1' }],
  ['a numeric csrf_token', { csrf_token: 7, credential_id: 'cred-1' }],
  ['a numeric credential_id', { csrf_token: 'tok-1', credential_id: 7 }],
  ['camelCase members', SESSION],
  ['null', null],
];

const WRONG_CEREMONIES: [string, unknown][] = [
  ['no ceremony_id', { options: CREATION_OPTIONS }],
  ['a numeric ceremony_id', { ceremony_id: 7, options: CREATION_OPTIONS }],
  ['null', null],
];

beforeEach(() => {
  refuse(webauthn.createCredential, 'createCredential');
  refuse(webauthn.getCredential, 'getCredential');
});

afterEach(() => {
  vi.unstubAllGlobals();
  dropSession();
});

describe('fetchSession', () => {
  it('resolves to the session the Hub reports', async () => {
    const { pending } = stubFetch([get('/v1/session', WIRE_SESSION)]);

    await expect(fetchSession()).resolves.toEqual(SESSION);
    expect(pending).toEqual([]);
  });

  it('maps 401 to null', async () => {
    stubFetch([get('/v1/session', { error: 'unauthenticated' }, 401)]);

    await expect(fetchSession()).resolves.toBeNull();
  });

  it.each([403, 404, 500])('rethrows a %i as a HubApiError', async (status) => {
    stubFetch([get('/v1/session', { error: 'internal' }, status)]);

    const error = await rejectionOf(fetchSession());

    expect(error.status).toBe(status);
    expect(error.path).toBe('/v1/session');
  });

  it('rethrows a request that got no response', async () => {
    stubFetch([]);

    const error = await rejectionOf(fetchSession());

    expect(error.status).toBeNull();
  });

  it.each(WRONG_SESSIONS)('rejects a session with %s', async (_shape, body) => {
    stubFetch([get('/v1/session', body)]);

    const error = await rejectionOf(fetchSession());

    expect(error.status).toBe(200);
  });

  it('hands its token to the requests that follow', async () => {
    const { sent } = stubFetch([get('/v1/session', WIRE_SESSION), post('/v1/session/end', {})]);

    await fetchSession();
    await signOut();

    expect(sent[1]?.csrfToken).toBe('tok-1');
  });
});

describe('signUp', () => {
  it('starts a ceremony, creates a credential over its options, and finishes with it', async () => {
    const { sent, pending } = stubFetch([
      post('/v1/register/start', { ceremony_id: 'cer-1', options: CREATION_OPTIONS }),
      post('/v1/register/finish', WIRE_SESSION),
    ]);
    webauthn.createCredential.mockImplementation(async () => {
      expect(sent.map((request) => request.path)).toEqual(['/v1/register/start']);
      return ATTESTATION;
    });

    await expect(signUp('laptop')).resolves.toEqual(SESSION);

    expect(pending).toEqual([]);
    expect(sent.map((request) => request.body)).toEqual([
      { label: 'laptop' },
      { ceremony_id: 'cer-1', credential: ATTESTATION },
    ]);
    expect(webauthn.createCredential.mock.calls).toEqual([[CREATION_OPTIONS]]);
  });

  it.each(WRONG_CEREMONIES)('rejects a start reply with %s before any prompt', async (_shape, body) => {
    stubFetch([post('/v1/register/start', body)]);

    const error = await rejectionOf(signUp('laptop'));

    expect(error.path).toBe('/v1/register/start');
    expect(error.status).toBe(200);
    expect(webauthn.createCredential).not.toHaveBeenCalled();
  });

  it.each(WRONG_SESSIONS)('rejects a finish reply with %s', async (_shape, body) => {
    stubFetch([
      post('/v1/register/start', { ceremony_id: 'cer-1', options: CREATION_OPTIONS }),
      post('/v1/register/finish', body),
    ]);
    webauthn.createCredential.mockResolvedValue(ATTESTATION);

    const error = await rejectionOf(signUp('laptop'));

    expect(error.path).toBe('/v1/register/finish');
    expect(error.status).toBe(200);
  });
});

describe('signIn', () => {
  it('starts a ceremony, gets a credential over its options, and finishes with it', async () => {
    const { sent, pending } = stubFetch([
      post('/v1/session/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
      post('/v1/session/finish', WIRE_SESSION),
    ]);
    webauthn.getCredential.mockImplementation(async () => {
      expect(sent.map((request) => request.path)).toEqual(['/v1/session/start']);
      return ASSERTION;
    });

    await expect(signIn()).resolves.toEqual(SESSION);

    expect(pending).toEqual([]);
    expect(sent.map((request) => request.body)).toEqual([{}, { ceremony_id: 'cer-2', credential: ASSERTION }]);
    expect(webauthn.getCredential.mock.calls).toEqual([[REQUEST_OPTIONS]]);
  });

  it.each(WRONG_CEREMONIES)('rejects a start reply with %s before any prompt', async (_shape, body) => {
    stubFetch([post('/v1/session/start', body)]);

    const error = await rejectionOf(signIn());

    expect(error.path).toBe('/v1/session/start');
    expect(error.status).toBe(200);
    expect(webauthn.getCredential).not.toHaveBeenCalled();
  });

  it.each(WRONG_SESSIONS)('rejects a finish reply with %s', async (_shape, body) => {
    stubFetch([
      post('/v1/session/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
      post('/v1/session/finish', body),
    ]);
    webauthn.getCredential.mockResolvedValue(ASSERTION);

    const error = await rejectionOf(signIn());

    expect(error.path).toBe('/v1/session/finish');
    expect(error.status).toBe(200);
  });

  it('hands its token to the requests that follow', async () => {
    const { sent } = stubFetch([
      post('/v1/session/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
      post('/v1/session/finish', WIRE_SESSION),
      post('/v1/session/end', {}),
    ]);
    webauthn.getCredential.mockResolvedValue(ASSERTION);

    await signIn();
    await signOut();

    expect(sent[2]?.csrfToken).toBe('tok-1');
  });
});

describe('signOut', () => {
  it('ends the session with the token the register response supplied, then drops it', async () => {
    const { sent, pending } = stubFetch([
      post('/v1/register/start', { ceremony_id: 'cer-1', options: CREATION_OPTIONS }),
      post('/v1/register/finish', WIRE_SESSION),
      post('/v1/session/end', {}),
      post('/v1/session/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
      post('/v1/session/finish', { csrf_token: 'tok-2', credential_id: 'cred-1' }),
    ]);
    webauthn.createCredential.mockResolvedValue(ATTESTATION);
    webauthn.getCredential.mockResolvedValue(ASSERTION);

    await signUp('laptop');
    await signOut();
    await signIn();

    expect(pending).toEqual([]);
    expect(sent[2]).toEqual({ path: '/v1/session/end', body: {}, csrfToken: 'tok-1' });
    expect(sent[3]?.csrfToken).toBeNull();
  });

  it('keeps the token when the Hub refuses, so the request can be sent again', async () => {
    setCsrfToken('tok-1');
    const { sent } = stubFetch([
      post('/v1/session/end', { error: 'internal' }, 500),
      post('/v1/session/end', {}),
    ]);

    const error = await rejectionOf(signOut());
    await signOut();

    expect(error.status).toBe(500);
    expect(sent[1]?.csrfToken).toBe('tok-1');
  });

  it.each([
    ['null', null],
    ['a string', 'ok'],
  ])('rejects a reply that is %s', async (_shape, body) => {
    stubFetch([post('/v1/session/end', body)]);

    const error = await rejectionOf(signOut());

    expect(error.status).toBe(200);
  });

  it('resolves on a 401, the session being over already, and drops the token', async () => {
    setCsrfToken('tok-1');
    const { sent, pending } = stubFetch([
      post('/v1/session/end', { error: 'unauthenticated' }, 401),
      post('/v1/session/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
      post('/v1/session/finish', WIRE_SESSION),
    ]);
    webauthn.getCredential.mockResolvedValue(ASSERTION);

    await expect(signOut()).resolves.toBeUndefined();
    await signIn();

    expect(pending).toEqual([]);
    expect(sent[0]?.csrfToken).toBe('tok-1');
    expect(sent[1]?.csrfToken).toBeNull();
  });

  it.each(['csrf_mismatch', 'csrf_required'])(
    'on a 403 %s, reads the session the cookie names and ends that one',
    async (code) => {
      setCsrfToken('tok-stale');
      const told = vi.fn();
      const stop = onSessionGone(told);
      const { sent, pending } = stubFetch([
        post('/v1/session/end', { error: code }, 403),
        get('/v1/session', { csrf_token: 'tok-cookie', credential_id: 'cred-1' }),
        post('/v1/session/end', {}),
        post('/v1/session/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
        post('/v1/session/finish', WIRE_SESSION),
      ]);
      webauthn.getCredential.mockResolvedValue(ASSERTION);

      await expect(signOut()).resolves.toBeUndefined();
      await signIn();

      expect(pending).toEqual([]);
      expect(sent.map((request) => [request.path, request.csrfToken])).toEqual([
        ['/v1/session/end', 'tok-stale'],
        ['/v1/session', null],
        ['/v1/session/end', 'tok-cookie'],
        ['/v1/session/start', null],
        ['/v1/session/finish', null],
      ]);
      expect(told).not.toHaveBeenCalled();
      stop();
    },
  );

  it('resolves when the session read after a CSRF refusal is 401, and drops the token', async () => {
    setCsrfToken('tok-stale');
    const { sent, pending } = stubFetch([
      post('/v1/session/end', { error: 'csrf_mismatch' }, 403),
      get('/v1/session', { error: 'unauthenticated' }, 401),
      post('/v1/session/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
      post('/v1/session/finish', WIRE_SESSION),
    ]);
    webauthn.getCredential.mockResolvedValue(ASSERTION);

    await expect(signOut()).resolves.toBeUndefined();
    await signIn();

    expect(pending).toEqual([]);
    expect(sent[2]?.csrfToken).toBeNull();
  });

  it('rejects when the retry is refused too, and does not ask a third time', async () => {
    setCsrfToken('tok-stale');
    const { sent, pending } = stubFetch([
      post('/v1/session/end', { error: 'csrf_mismatch' }, 403),
      get('/v1/session', { csrf_token: 'tok-cookie', credential_id: 'cred-1' }),
      post('/v1/session/end', { error: 'csrf_mismatch' }, 403),
    ]);

    const error = await rejectionOf(signOut());

    expect(error.status).toBe(403);
    expect(error.code).toBe('csrf_mismatch');
    expect(pending).toEqual([]);
    expect(sent).toHaveLength(3);
  });

  it('rejects when the session read after a CSRF refusal is not answered', async () => {
    setCsrfToken('tok-stale');
    const { pending } = stubFetch([
      post('/v1/session/end', { error: 'csrf_mismatch' }, 403),
      get('/v1/session', { error: 'internal' }, 500),
    ]);

    const error = await rejectionOf(signOut());

    expect(error.path).toBe('/v1/session');
    expect(error.status).toBe(500);
    expect(pending).toEqual([]);
  });

  it.each([
    [403, 'origin_mismatch'],
    [403, null],
  ])('rejects a %i %s without reading the session', async (status, code) => {
    setCsrfToken('tok-1');
    const { sent } = stubFetch([post('/v1/session/end', { error: code }, status)]);

    const error = await rejectionOf(signOut());

    expect(error.status).toBe(status);
    expect(sent).toHaveLength(1);
  });
});

describe('postInSession', () => {
  const PATH = '/v1/passkeys/add/start';
  const BODY = { label: 'phone' };
  const STALE = { csrf_token: 'tok-stale', credential_id: 'cred-1' };
  const REREAD = { csrf_token: 'tok-cookie', credential_id: 'cred-1' };
  const isReply: Guard<{ ok: boolean }> = (value): value is { ok: boolean } =>
    typeof value === 'object' && value !== null && Reflect.get(value, 'ok') === true;

  /** Signs this tab in as `cred-1` holding `tok-stale`, then scripts what follows. */
  async function signedInThen(script: Exchange[]) {
    stubFetch([get('/v1/session', STALE)]);
    await fetchSession();
    const told = vi.fn();
    const stop = onSessionGone(told);
    return { ...stubFetch(script), told, stop };
  }

  it('sends the request once when it is answered', async () => {
    const { sent, pending, stop } = await signedInThen([post(PATH, { ok: true })]);

    await expect(postInSession(PATH, BODY, isReply)).resolves.toEqual({ ok: true });

    expect(pending).toEqual([]);
    expect(sent).toEqual([{ path: PATH, body: BODY, csrfToken: 'tok-stale' }]);
    stop();
  });

  it.each(['csrf_mismatch', 'csrf_required'])(
    'on a 403 %s, resends with the token of a session read that names the same passkey',
    async (code) => {
      const { sent, pending, told, stop } = await signedInThen([
        post(PATH, { error: code }, 403),
        get('/v1/session', REREAD),
        post(PATH, { ok: true }),
      ]);

      await expect(postInSession(PATH, BODY, isReply)).resolves.toEqual({ ok: true });

      expect(pending).toEqual([]);
      expect(sent).toEqual([
        { path: PATH, body: BODY, csrfToken: 'tok-stale' },
        { path: '/v1/session', body: undefined, csrfToken: null },
        { path: PATH, body: BODY, csrfToken: 'tok-cookie' },
      ]);
      expect(told).not.toHaveBeenCalled();
      stop();
    },
  );

  it('rejects with the second refusal when the resent request is refused too, and asks no third time', async () => {
    const { sent, pending, told, stop } = await signedInThen([
      post(PATH, { error: 'csrf_mismatch' }, 403),
      get('/v1/session', REREAD),
      post(PATH, { error: 'csrf_required' }, 403),
    ]);

    const error = await rejectionOf(postInSession(PATH, BODY, isReply));

    expect(error.status).toBe(403);
    expect(error.code).toBe('csrf_required');
    expect(pending).toEqual([]);
    expect(sent).toHaveLength(3);
    expect(told).not.toHaveBeenCalled();
    stop();
  });

  it('does not resend into a session read that names another passkey, and drops the session', async () => {
    const { sent, pending, told, stop } = await signedInThen([
      post(PATH, { error: 'csrf_mismatch' }, 403),
      get('/v1/session', { csrf_token: 'tok-other', credential_id: 'cred-9' }),
      post('/v1/session/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
    ]);

    const error = await rejectionOf(postInSession(PATH, BODY, isReply));
    await signIn().catch(() => undefined);

    expect(error.path).toBe(PATH);
    expect(error.status).toBe(403);
    expect(error.code).toBe('csrf_mismatch');
    expect(pending).toEqual([]);
    expect(sent.map((request) => [request.path, request.csrfToken])).toEqual([
      [PATH, 'tok-stale'],
      ['/v1/session', null],
      ['/v1/session/start', null],
    ]);
    expect(told).toHaveBeenCalledTimes(1);
    stop();
  });

  it('does not resend when the session read is 401, the session being dropped by that answer', async () => {
    const { sent, pending, told, stop } = await signedInThen([
      post(PATH, { error: 'csrf_mismatch' }, 403),
      get('/v1/session', { error: 'unauthenticated' }, 401),
    ]);

    const error = await rejectionOf(postInSession(PATH, BODY, isReply));

    expect(error.path).toBe(PATH);
    expect(error.code).toBe('csrf_mismatch');
    expect(pending).toEqual([]);
    expect(sent).toHaveLength(2);
    expect(told).toHaveBeenCalledTimes(1);
    stop();
  });

  it('does not resend into any session once this tab has signed out', async () => {
    const { sent, told, stop } = await signedInThen([
      post('/v1/session/end', {}),
      post(PATH, { error: 'csrf_required' }, 403),
      get('/v1/session', REREAD),
    ]);
    await signOut();

    const error = await rejectionOf(postInSession(PATH, BODY, isReply));

    expect(error.code).toBe('csrf_required');
    expect(sent).toHaveLength(3);
    expect(told).toHaveBeenCalledTimes(1);
    stop();
  });

  it('does not resend into any session once the one this tab held was dropped', async () => {
    const { sent, told, stop } = await signedInThen([
      post(PATH, { error: 'csrf_required' }, 403),
      get('/v1/session', REREAD),
    ]);
    dropSession();

    const error = await rejectionOf(postInSession(PATH, BODY, isReply));

    expect(error.code).toBe('csrf_required');
    expect(sent).toHaveLength(2);
    expect(told).toHaveBeenCalledTimes(2);
    stop();
  });

  it('rejects with the failed session read, and resends nothing', async () => {
    const { pending, told, stop } = await signedInThen([
      post(PATH, { error: 'csrf_mismatch' }, 403),
      get('/v1/session', { error: 'internal' }, 500),
    ]);

    const error = await rejectionOf(postInSession(PATH, BODY, isReply));

    expect(error.path).toBe('/v1/session');
    expect(error.status).toBe(500);
    expect(pending).toEqual([]);
    expect(told).not.toHaveBeenCalled();
    stop();
  });

  it.each([
    [403, 'origin_mismatch'],
    [403, 'self_endorsement'],
    [403, null],
    [409, 'last_passkey'],
    [500, 'internal'],
  ])('rejects a %i %s without reading the session', async (status, code) => {
    const { sent, stop } = await signedInThen([post(PATH, { error: code }, status)]);

    const error = await rejectionOf(postInSession(PATH, BODY, isReply));

    expect(error.status).toBe(status);
    expect(error.code).toBe(code);
    expect(sent).toHaveLength(1);
    stop();
  });
});

describe('sessionStillHeld', () => {
  async function signedInThen(script: Exchange[]) {
    stubFetch([get('/v1/session', WIRE_SESSION)]);
    await fetchSession();
    const told = vi.fn();
    const stop = onSessionGone(told);
    return { ...stubFetch(script), told, stop };
  }

  it('is true for a session that names the passkey this tab signed in with, and takes its token', async () => {
    const { sent, told, stop } = await signedInThen([
      get('/v1/session', { csrf_token: 'tok-2', credential_id: 'cred-1' }),
      post('/v1/session/end', {}),
    ]);

    await expect(sessionStillHeld()).resolves.toBe(true);
    await signOut();

    expect(sent[1]?.csrfToken).toBe('tok-2');
    expect(told).not.toHaveBeenCalled();
    stop();
  });

  it('is false for a session that names another passkey, and tells the listener', async () => {
    const { told, stop } = await signedInThen([
      get('/v1/session', { csrf_token: 'tok-2', credential_id: 'cred-9' }),
    ]);

    await expect(sessionStillHeld()).resolves.toBe(false);

    expect(told).toHaveBeenCalledTimes(1);
    stop();
  });

  it('is false on a 401, which has told the listener already', async () => {
    const { told, stop } = await signedInThen([get('/v1/session', { error: 'unauthenticated' }, 401)]);

    await expect(sessionStillHeld()).resolves.toBe(false);

    expect(told).toHaveBeenCalledTimes(1);
    stop();
  });

  it('rejects, and tells nobody, when the read says nothing about the session', async () => {
    const { told, stop } = await signedInThen([get('/v1/session', { error: 'internal' }, 500)]);

    const error = await rejectionOf(sessionStillHeld());

    expect(error.status).toBe(500);
    expect(told).not.toHaveBeenCalled();
    stop();
  });
});
