import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { dropSession, HubApiError, onSessionGone, postJson, setCsrfToken } from './api';
import { fetchSession } from './auth';
import { addPasskey, endorsePasskey, listPasskeys, revokePasskey } from './passkeys';

const webauthn = vi.hoisted(() => ({
  createCredential: vi.fn<(options: unknown) => Promise<unknown>>(),
  getAssertion: vi.fn<(options: unknown) => Promise<unknown>>(),
}));

vi.mock('./webauthn', () => webauthn);

/** An `Error` for `answer` is a request that got no response. */
type Exchange = { method: 'GET' | 'POST'; path: string; answer: Response | Error };

type Sent = { path: string; body: unknown };

/**
 * Every request a double was not told about. The throw alone is not enough
 * here: `revokePasskey` reads the session on its way out of a failure and
 * discards how that read went, so an unscripted one would pass unseen.
 */
let unscripted: string[] = [];

/** The scripted double of `auth.test.ts`: a request that is not the next one in `script` throws. */
function stubFetch(script: Exchange[]) {
  const pending = [...script];
  const sent: Sent[] = [];
  const tokens: (string | null)[] = [];
  const double = vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
    const method = init?.method ?? 'GET';
    const next = pending.shift();
    if (next === undefined || next.path !== input || next.method !== method) {
      unscripted.push(`${method} ${String(input)}`);
      throw new Error(`fetch double was not told about ${method} ${String(input)} here`);
    }
    sent.push({
      path: next.path,
      body: typeof init?.body === 'string' ? (JSON.parse(init.body) as unknown) : undefined,
    });
    tokens.push(new Headers(init?.headers).get('X-CSRF-Token'));
    if (next.answer instanceof Error) throw next.answer;
    return next.answer;
  });
  vi.stubGlobal('fetch', double);
  return { sent, pending, tokens };
}

function post(path: string, body: unknown, status = 200): Exchange {
  return { method: 'POST', path, answer: json(body, status) };
}

function get(path: string, body: unknown, status = 200): Exchange {
  return { method: 'GET', path, answer: json(body, status) };
}

function unanswered(path: string): Exchange {
  return { method: 'POST', path, answer: new TypeError('Failed to fetch') };
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

function refuse(double: typeof webauthn.createCredential, name: string): void {
  double.mockImplementation(async () => {
    throw new Error(`${name} double was not told to answer`);
  });
}

const CREATION_OPTIONS = { publicKey: { challenge: 'Y3JlYXRl' } };
const REQUEST_OPTIONS = { publicKey: { challenge: 'Z2V0' } };
const ATTESTATION = { id: 'cred-2', response: { attestationObject: 'YXR0' } };
const ASSERTION = {
  credential_id: 'cred-1',
  authenticator_data: 'YXV0aA',
  client_data_json: 'Y2xpZW50',
  signature: 'c2ln',
};

const WIRE_PASSKEY = {
  credential_id: 'cred-2',
  label: 'phone',
  status: 'pending',
  created_at: 1_700_000_000,
  current: false,
};
const PASSKEY = {
  credentialId: 'cred-2',
  label: 'phone',
  status: 'pending',
  createdAt: 1_700_000_000,
  current: false,
};

const WRONG_PASSKEYS: [string, unknown][] = [
  ['no credential_id', { ...WIRE_PASSKEY, credential_id: undefined }],
  ['a numeric credential_id', { ...WIRE_PASSKEY, credential_id: 7 }],
  ['no label', { ...WIRE_PASSKEY, label: undefined }],
  ['a numeric label', { ...WIRE_PASSKEY, label: 7 }],
  ['no status', { ...WIRE_PASSKEY, status: undefined }],
  ['an unknown status', { ...WIRE_PASSKEY, status: 'endorsed' }],
  ['a status in another case', { ...WIRE_PASSKEY, status: 'Trusted' }],
  ['no created_at', { ...WIRE_PASSKEY, created_at: undefined }],
  ['a string created_at', { ...WIRE_PASSKEY, created_at: '1700000000' }],
  ['no current', { ...WIRE_PASSKEY, current: undefined }],
  ['a string current', { ...WIRE_PASSKEY, current: 'false' }],
  ['camelCase members', PASSKEY],
  ['null', null],
];

const WRONG_CEREMONIES: [string, unknown][] = [
  ['no ceremony_id', { options: REQUEST_OPTIONS }],
  ['a numeric ceremony_id', { ceremony_id: 7, options: REQUEST_OPTIONS }],
  ['null', null],
];

beforeEach(() => {
  refuse(webauthn.createCredential, 'createCredential');
  refuse(webauthn.getAssertion, 'getAssertion');
});

afterEach(() => {
  vi.unstubAllGlobals();
  dropSession();
  const strays = unscripted;
  unscripted = [];
  // Soft: a throw here would skip the setup file's own `afterEach`, and its
  // `clearAllMocks`, turning one stray request into a failure of the next test.
  expect.soft(strays).toEqual([]);
});

describe('listPasskeys', () => {
  it('maps every wire passkey to its TypeScript shape, in the order sent', async () => {
    const { pending } = stubFetch([
      get('/v1/passkeys', {
        passkeys: [
          {
            credential_id: 'cred-1',
            label: 'laptop',
            status: 'trusted',
            created_at: 1_600_000_000,
            current: true,
          },
          WIRE_PASSKEY,
          {
            credential_id: 'cred-3',
            label: 'old key',
            status: 'revoked',
            created_at: 1_500_000_000,
            current: false,
          },
        ],
      }),
    ]);

    await expect(listPasskeys()).resolves.toEqual([
      {
        credentialId: 'cred-1',
        label: 'laptop',
        status: 'trusted',
        createdAt: 1_600_000_000,
        current: true,
      },
      PASSKEY,
      {
        credentialId: 'cred-3',
        label: 'old key',
        status: 'revoked',
        createdAt: 1_500_000_000,
        current: false,
      },
    ]);
    expect(pending).toEqual([]);
  });

  it('resolves to an empty list', async () => {
    stubFetch([get('/v1/passkeys', { passkeys: [] })]);

    await expect(listPasskeys()).resolves.toEqual([]);
  });

  it.each(WRONG_PASSKEYS)('rejects a list holding a passkey with %s', async (_shape, passkey) => {
    stubFetch([get('/v1/passkeys', { passkeys: [WIRE_PASSKEY, passkey] })]);

    const error = await rejectionOf(listPasskeys());

    expect(error.path).toBe('/v1/passkeys');
    expect(error.status).toBe(200);
  });

  it.each([
    ['no passkeys member', {}],
    ['one passkey where the list belongs', { passkeys: WIRE_PASSKEY }],
    ['a bare array', [WIRE_PASSKEY]],
    ['null', null],
  ])('rejects a reply with %s', async (_shape, body) => {
    stubFetch([get('/v1/passkeys', body)]);

    const error = await rejectionOf(listPasskeys());

    expect(error.status).toBe(200);
  });

  it('surfaces a Hub refusal with its code', async () => {
    stubFetch([get('/v1/passkeys', { error: 'unauthenticated' }, 401)]);

    const error = await rejectionOf(listPasskeys());

    expect(error.status).toBe(401);
    expect(error.code).toBe('unauthenticated');
  });
});

describe('addPasskey', () => {
  it('starts a ceremony, creates a credential over its options, and finishes with it', async () => {
    const { sent, pending } = stubFetch([
      post('/v1/passkeys/add/start', { ceremony_id: 'cer-1', options: CREATION_OPTIONS }),
      post('/v1/passkeys/add/finish', { passkey: WIRE_PASSKEY }),
    ]);
    webauthn.createCredential.mockImplementation(async () => {
      expect(sent.map((request) => request.path)).toEqual(['/v1/passkeys/add/start']);
      return ATTESTATION;
    });

    await expect(addPasskey('phone')).resolves.toEqual(PASSKEY);

    expect(pending).toEqual([]);
    expect(sent.map((request) => request.body)).toEqual([
      { label: 'phone' },
      { ceremony_id: 'cer-1', credential: ATTESTATION },
    ]);
    expect(webauthn.createCredential.mock.calls).toEqual([[CREATION_OPTIONS]]);
  });

  it.each(WRONG_CEREMONIES)('rejects a start reply with %s before any prompt', async (_shape, body) => {
    stubFetch([post('/v1/passkeys/add/start', body)]);

    const error = await rejectionOf(addPasskey('phone'));

    expect(error.path).toBe('/v1/passkeys/add/start');
    expect(error.status).toBe(200);
    expect(webauthn.createCredential).not.toHaveBeenCalled();
  });

  it.each(WRONG_PASSKEYS)('rejects a finish reply whose passkey has %s', async (_shape, passkey) => {
    stubFetch([
      post('/v1/passkeys/add/start', { ceremony_id: 'cer-1', options: CREATION_OPTIONS }),
      post('/v1/passkeys/add/finish', { passkey }),
    ]);
    webauthn.createCredential.mockResolvedValue(ATTESTATION);

    const error = await rejectionOf(addPasskey('phone'));

    expect(error.path).toBe('/v1/passkeys/add/finish');
    expect(error.status).toBe(200);
  });

  it('rejects a finish reply that is the passkey itself, unwrapped', async () => {
    stubFetch([
      post('/v1/passkeys/add/start', { ceremony_id: 'cer-1', options: CREATION_OPTIONS }),
      post('/v1/passkeys/add/finish', WIRE_PASSKEY),
    ]);
    webauthn.createCredential.mockResolvedValue(ATTESTATION);

    const error = await rejectionOf(addPasskey('phone'));

    expect(error.status).toBe(200);
  });

  it('surfaces a Hub refusal with its code and prompts for nothing', async () => {
    stubFetch([post('/v1/passkeys/add/start', { error: 'too_many_ceremonies' }, 503)]);

    const error = await rejectionOf(addPasskey('phone'));

    expect(error.status).toBe(503);
    expect(error.code).toBe('too_many_ceremonies');
    expect(webauthn.createCredential).not.toHaveBeenCalled();
  });
});

describe.each([
  {
    name: 'endorsePasskey',
    change: endorsePasskey,
    route: '/v1/passkeys/endorse',
    status: 'trusted',
    afterUnreadFinish: (): Exchange[] => [],
  },
  {
    name: 'revokePasskey',
    change: revokePasskey,
    route: '/v1/passkeys/revoke',
    status: 'revoked',
    afterUnreadFinish: (): Exchange[] => [get('/v1/session', { error: 'unauthenticated' }, 401)],
  },
])('$name', ({ change, route, status, afterUnreadFinish }) => {
  const start = `${route}/start`;
  const finish = `${route}/finish`;
  const started = { ceremony_id: 'cer-3', options: REQUEST_OPTIONS };

  it('starts a ceremony over the target, asserts over its options, and finishes with the assertion', async () => {
    const { sent, pending } = stubFetch([
      post(start, started),
      post(finish, { passkey: { ...WIRE_PASSKEY, status } }),
    ]);
    webauthn.getAssertion.mockImplementation(async () => {
      expect(sent.map((request) => request.path)).toEqual([start]);
      return ASSERTION;
    });

    await expect(change('cred-2')).resolves.toEqual({ ...PASSKEY, status });

    expect(pending).toEqual([]);
    expect(sent).toEqual([
      { path: start, body: { credential_id: 'cred-2' } },
      {
        path: finish,
        body: {
          ceremony_id: 'cer-3',
          assertion: {
            credential_id: 'cred-1',
            authenticator_data: 'YXV0aA',
            client_data_json: 'Y2xpZW50',
            signature: 'c2ln',
          },
        },
      },
    ]);
    expect(webauthn.getAssertion.mock.calls).toEqual([[REQUEST_OPTIONS]]);
  });

  it.each(WRONG_CEREMONIES)('rejects a start reply with %s before any prompt', async (_shape, body) => {
    stubFetch([post(start, body)]);

    const error = await rejectionOf(change('cred-2'));

    expect(error.path).toBe(start);
    expect(error.status).toBe(200);
    expect(webauthn.getAssertion).not.toHaveBeenCalled();
  });

  it.each(WRONG_PASSKEYS)('rejects a finish reply whose passkey has %s', async (_shape, passkey) => {
    const { pending } = stubFetch([
      post(start, started),
      post(finish, { passkey }),
      ...afterUnreadFinish(),
    ]);
    webauthn.getAssertion.mockResolvedValue(ASSERTION);

    const error = await rejectionOf(change('cred-2'));

    expect(error.path).toBe(finish);
    expect(error.status).toBe(200);
    expect(pending).toEqual([]);
  });

  it.each([
    [403, 'self_endorsement'],
    [409, 'last_passkey'],
  ])('surfaces a %i %s from start with its code and prompts for nothing', async (refusal, code) => {
    stubFetch([post(start, { error: code }, refusal)]);

    const error = await rejectionOf(change('cred-2'));

    expect(error.path).toBe(start);
    expect(error.status).toBe(refusal);
    expect(error.code).toBe(code);
    expect(webauthn.getAssertion).not.toHaveBeenCalled();
  });

  it.each([
    [403, 'self_endorsement'],
    [409, 'last_passkey'],
  ])('surfaces a %i %s from finish with its code', async (refusal, code) => {
    stubFetch([post(start, started), post(finish, { error: code }, refusal)]);
    webauthn.getAssertion.mockResolvedValue(ASSERTION);

    const error = await rejectionOf(change('cred-2'));

    expect(error.path).toBe(finish);
    expect(error.status).toBe(refusal);
    expect(error.code).toBe(code);
  });
});

describe('a trust change answered about the passkey this session signed in with', () => {
  const started = { ceremony_id: 'cer-3', options: REQUEST_OPTIONS };
  const isAnything = (_value: unknown): _value is unknown => true;

  /** The token `postJson` would send next, read off a request made for that purpose. */
  async function heldToken(): Promise<string | null> {
    let held: string | null = null;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (_input: unknown, init?: RequestInit) => {
        held = new Headers(init?.headers).get('X-CSRF-Token');
        return json({}, 200);
      }),
    );
    await postJson('/v1/probe', {}, isAnything);
    return held;
  }

  async function changeAnswered(
    change: typeof revokePasskey,
    route: string,
    passkey: typeof WIRE_PASSKEY,
  ) {
    const told = vi.fn();
    const stop = onSessionGone(told);
    setCsrfToken('tok-1');
    stubFetch([post(`${route}/start`, started), post(`${route}/finish`, { passkey })]);
    webauthn.getAssertion.mockResolvedValue(ASSERTION);

    const changed = await change(passkey.credential_id);

    stop();
    return { changed, told, token: await heldToken() };
  }

  it('revoked: the session is gone, so the token is dropped and the listener told', async () => {
    const { changed, told, token } = await changeAnswered(revokePasskey, '/v1/passkeys/revoke', {
      ...WIRE_PASSKEY,
      status: 'revoked',
      current: true,
    });

    expect(changed).toEqual({ ...PASSKEY, status: 'revoked', current: true });
    expect(told).toHaveBeenCalledTimes(1);
    expect(token).toBeNull();
  });

  it('revoked, but another passkey: the session stands', async () => {
    const { told, token } = await changeAnswered(revokePasskey, '/v1/passkeys/revoke', {
      ...WIRE_PASSKEY,
      status: 'revoked',
      current: false,
    });

    expect(told).not.toHaveBeenCalled();
    expect(token).toBe('tok-1');
  });

  it('endorsed: the session stands', async () => {
    const { told, token } = await changeAnswered(endorsePasskey, '/v1/passkeys/endorse', {
      ...WIRE_PASSKEY,
      status: 'trusted',
      current: true,
    });

    expect(told).not.toHaveBeenCalled();
    expect(token).toBe('tok-1');
  });
});

const HELD = { csrf_token: 'tok-stale', credential_id: 'cred-1' };
const REREAD = { csrf_token: 'tok-cookie', credential_id: 'cred-1' };

/** Signs this tab in as `cred-1` holding `tok-stale`, then scripts what follows. */
async function signedInThen(script: Exchange[]) {
  stubFetch([get('/v1/session', HELD)]);
  await fetchSession();
  const told = vi.fn();
  const stop = onSessionGone(told);
  return { ...stubFetch(script), told, stop };
}

describe.each([
  {
    name: 'addPasskey',
    act: () => addPasskey('phone'),
    start: '/v1/passkeys/add/start',
    finish: '/v1/passkeys/add/finish',
    options: CREATION_OPTIONS,
  },
  {
    name: 'endorsePasskey',
    act: () => endorsePasskey('cred-2'),
    start: '/v1/passkeys/endorse/start',
    finish: '/v1/passkeys/endorse/finish',
    options: REQUEST_OPTIONS,
  },
  {
    name: 'revokePasskey',
    act: () => revokePasskey('cred-2'),
    start: '/v1/passkeys/revoke/start',
    finish: '/v1/passkeys/revoke/finish',
    options: REQUEST_OPTIONS,
  },
])('$name from a tab whose CSRF token went stale', ({ act, start, finish, options }) => {
  const started = () => post(start, { ceremony_id: 'cer-4', options });
  const finished = () => post(finish, { passkey: WIRE_PASSKEY });
  const refused = (path: string) => post(path, { error: 'csrf_mismatch' }, 403);

  beforeEach(() => {
    webauthn.createCredential.mockResolvedValue(ATTESTATION);
    webauthn.getAssertion.mockResolvedValue(ASSERTION);
  });

  it('resends a refused start with the token of the session read, and carries on', async () => {
    const { sent, pending, tokens, told, stop } = await signedInThen([
      refused(start),
      get('/v1/session', REREAD),
      started(),
      finished(),
    ]);

    await expect(act()).resolves.toEqual(PASSKEY);

    expect(pending).toEqual([]);
    expect(sent.map((request) => request.path)).toEqual([start, '/v1/session', start, finish]);
    expect(tokens).toEqual(['tok-stale', null, 'tok-cookie', 'tok-cookie']);
    expect(sent[2]?.body).toEqual(sent[0]?.body);
    expect(told).not.toHaveBeenCalled();
    stop();
  });

  it('resends a refused finish with the same body, and prompts once', async () => {
    const { sent, pending, tokens, told, stop } = await signedInThen([
      started(),
      refused(finish),
      get('/v1/session', REREAD),
      finished(),
    ]);

    await expect(act()).resolves.toEqual(PASSKEY);

    expect(pending).toEqual([]);
    expect(sent.map((request) => request.path)).toEqual([start, finish, '/v1/session', finish]);
    expect(tokens).toEqual(['tok-stale', 'tok-stale', null, 'tok-cookie']);
    expect(sent[3]?.body).toEqual(sent[1]?.body);
    expect(webauthn.createCredential.mock.calls.length + webauthn.getAssertion.mock.calls.length).toBe(1);
    expect(told).not.toHaveBeenCalled();
    stop();
  });

  it('rejects with the second refusal after exactly one resend', async () => {
    const { sent, pending, stop } = await signedInThen([
      refused(start),
      get('/v1/session', REREAD),
      refused(start),
    ]);

    const error = await rejectionOf(act());

    expect(error.status).toBe(403);
    expect(error.code).toBe('csrf_mismatch');
    expect(pending).toEqual([]);
    expect(sent.map((request) => request.path)).toEqual([start, '/v1/session', start]);
    stop();
  });

  it('resends nothing into a session of another passkey, and tells the listener once', async () => {
    const { sent, pending, told, stop } = await signedInThen([
      refused(start),
      get('/v1/session', { csrf_token: 'tok-other', credential_id: 'cred-9' }),
    ]);

    const error = await rejectionOf(act());

    expect(error.path).toBe(start);
    expect(error.code).toBe('csrf_mismatch');
    expect(pending).toEqual([]);
    expect(sent.map((request) => request.path)).toEqual([start, '/v1/session']);
    expect(told).toHaveBeenCalledTimes(1);
    stop();
  });

  it('resends nothing when the session read is 401, and tells the listener once', async () => {
    const { sent, pending, told, stop } = await signedInThen([
      refused(start),
      get('/v1/session', { error: 'unauthenticated' }, 401),
    ]);

    const error = await rejectionOf(act());

    expect(error.path).toBe(start);
    expect(error.code).toBe('csrf_mismatch');
    expect(pending).toEqual([]);
    expect(sent.map((request) => request.path)).toEqual([start, '/v1/session']);
    expect(told).toHaveBeenCalledTimes(1);
    stop();
  });
});

describe('a revocation whose finish may have been committed unanswered', () => {
  const start = '/v1/passkeys/revoke/start';
  const finish = '/v1/passkeys/revoke/finish';
  const started = () => post(start, { ceremony_id: 'cer-5', options: REQUEST_OPTIONS });

  beforeEach(() => {
    webauthn.getAssertion.mockResolvedValue(ASSERTION);
  });

  const LOST: [string, () => Exchange, number | null][] = [
    ['got no response', () => unanswered(finish), null],
    ['was answered 500', () => post(finish, { error: 'internal' }, 500), 500],
    ['was answered 200 with a body that is not a passkey', () => post(finish, {}), 200],
  ];

  it.each(LOST)('finish %s, session read 401: the listener is told and the failure rejected', async (_how, lost, status) => {
    const { pending, told, stop } = await signedInThen([
      started(),
      lost(),
      get('/v1/session', { error: 'unauthenticated' }, 401),
    ]);

    const error = await rejectionOf(revokePasskey('cred-1'));

    expect(error.path).toBe(finish);
    expect(error.status).toBe(status);
    expect(pending).toEqual([]);
    expect(told).toHaveBeenCalledTimes(1);
    stop();
  });

  it.each(LOST)('finish %s, session still live: the listener is not told and the failure rejected', async (_how, lost, status) => {
    const { pending, told, stop } = await signedInThen([started(), lost(), get('/v1/session', REREAD)]);

    const error = await rejectionOf(revokePasskey('cred-2'));

    expect(error.path).toBe(finish);
    expect(error.status).toBe(status);
    expect(pending).toEqual([]);
    expect(told).not.toHaveBeenCalled();
    stop();
  });

  it('rejects with the finish failure when the session read fails too', async () => {
    const { pending, told, stop } = await signedInThen([
      started(),
      unanswered(finish),
      get('/v1/session', { error: 'internal' }, 500),
    ]);

    const error = await rejectionOf(revokePasskey('cred-1'));

    expect(error.path).toBe(finish);
    expect(error.status).toBeNull();
    expect(pending).toEqual([]);
    expect(told).not.toHaveBeenCalled();
    stop();
  });

  it('tells the listener when the session read names another passkey', async () => {
    const { told, stop } = await signedInThen([
      started(),
      unanswered(finish),
      get('/v1/session', { csrf_token: 'tok-other', credential_id: 'cred-9' }),
    ]);

    const error = await rejectionOf(revokePasskey('cred-1'));

    expect(error.path).toBe(finish);
    expect(told).toHaveBeenCalledTimes(1);
    stop();
  });

  it.each([
    [409, 'last_passkey'],
    [403, 'self_endorsement'],
    [400, 'ceremony_expired'],
  ])('reads no session after a %i %s from finish', async (status, code) => {
    const { sent, told, stop } = await signedInThen([started(), post(finish, { error: code }, status)]);

    const error = await rejectionOf(revokePasskey('cred-1'));

    expect(error.code).toBe(code);
    expect(sent.map((request) => request.path)).toEqual([start, finish]);
    expect(told).not.toHaveBeenCalled();
    stop();
  });

  it('reads no session when it is the start that failed', async () => {
    const { sent, stop } = await signedInThen([post(start, { error: 'internal' }, 500)]);

    const error = await rejectionOf(revokePasskey('cred-1'));

    expect(error.status).toBe(500);
    expect(sent.map((request) => request.path)).toEqual([start]);
    stop();
  });

  it('reads no session for an endorsement, which ends none', async () => {
    const endorse = '/v1/passkeys/endorse';
    const { sent, stop } = await signedInThen([
      post(`${endorse}/start`, { ceremony_id: 'cer-5', options: REQUEST_OPTIONS }),
      post(`${endorse}/finish`, { error: 'internal' }, 500),
    ]);

    const error = await rejectionOf(endorsePasskey('cred-2'));

    expect(error.status).toBe(500);
    expect(sent.map((request) => request.path)).toEqual([`${endorse}/start`, `${endorse}/finish`]);
    stop();
  });
});
