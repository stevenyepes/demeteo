import { afterEach, describe, expect, it, vi } from 'vitest';

import { dropSession, HubApiError, onSessionGone } from './api';
import { fetchSession } from './auth';
import { approvePairing, denyPairing, leaveFor, startPairing } from './pairing';

type Exchange = { method: 'GET' | 'POST'; path: string; answer: Response };

type Sent = { path: string; body: unknown; csrfToken: string | null };

/** The scripted double of `auth.test.ts`: a request that is not the next one in `script` throws. */
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

/** Stands where `window.location` does, so a navigation is a recorded call and not jsdom's refusal of one. */
function stubLocation() {
  const assign = vi.fn<(url: string) => void>();
  vi.stubGlobal('location', { assign });
  return assign;
}

const START = '/v1/pairing/start';
const APPROVE = '/v1/pairing/approve';
const DENY = '/v1/pairing/deny';

const SEGMENT = 'eyJjbGllbnQiOiJkZXNrdG9wIn0';

const WIRE_PROMPT = {
  pairing_id: 'pair-1',
  fingerprint: 'SHA256:0ZcO CORZ NYy- DWpq',
  instance_name: 'workstation',
  os: 'linux',
  arch: 'x86_64',
  app_version: '1.4.0',
  scopes: ['read', 'spend', 'gates'],
  expires_at: 1_700_000_600,
};
const PROMPT = {
  pairingId: 'pair-1',
  fingerprint: 'SHA256:0ZcO CORZ NYy- DWpq',
  instanceName: 'workstation',
  os: 'linux',
  arch: 'x86_64',
  appVersion: '1.4.0',
  scopes: ['read', 'spend', 'gates'],
  expiresAt: 1_700_000_600,
};

const WRONG_PROMPTS: [string, unknown][] = [
  ['no pairing_id', { ...WIRE_PROMPT, pairing_id: undefined }],
  ['a numeric pairing_id', { ...WIRE_PROMPT, pairing_id: 7 }],
  ['no fingerprint', { ...WIRE_PROMPT, fingerprint: undefined }],
  ['a numeric fingerprint', { ...WIRE_PROMPT, fingerprint: 7 }],
  ['no instance_name', { ...WIRE_PROMPT, instance_name: undefined }],
  ['a numeric instance_name', { ...WIRE_PROMPT, instance_name: 7 }],
  ['no os', { ...WIRE_PROMPT, os: undefined }],
  ['a numeric os', { ...WIRE_PROMPT, os: 7 }],
  ['no arch', { ...WIRE_PROMPT, arch: undefined }],
  ['a numeric arch', { ...WIRE_PROMPT, arch: 7 }],
  ['no app_version', { ...WIRE_PROMPT, app_version: undefined }],
  ['a numeric app_version', { ...WIRE_PROMPT, app_version: 7 }],
  ['no scopes', { ...WIRE_PROMPT, scopes: undefined }],
  ['one scope where the list belongs', { ...WIRE_PROMPT, scopes: 'read' }],
  ['a space-separated scope list', { ...WIRE_PROMPT, scopes: 'read spend' }],
  ['an unknown scope', { ...WIRE_PROMPT, scopes: ['read', 'admin'] }],
  ['a scope in another case', { ...WIRE_PROMPT, scopes: ['Read'] }],
  ['a numeric scope', { ...WIRE_PROMPT, scopes: ['read', 7] }],
  ['no expires_at', { ...WIRE_PROMPT, expires_at: undefined }],
  ['a string expires_at', { ...WIRE_PROMPT, expires_at: '1700000600' }],
  ['camelCase members', PROMPT],
  ['a bare array', [WIRE_PROMPT]],
  ['null', null],
];

const APPROVED = 'http://127.0.0.1:49152/callback?code=Zm9v-YmFy_0.~&state=xyz-123';
const DENIED = 'http://127.0.0.1:49152/callback?error=access_denied&state=xyz-123';

const LOOPBACK_REDIRECTS: [string, string][] = [
  ['an authorization code', APPROVED],
  ['a denial', DENIED],
  ['the lowest port', 'http://127.0.0.1:1/?code=c&state=s'],
  ['the highest port', 'http://127.0.0.1:65535/oauth/callback?code=c&state=s'],
  ['a nested path', 'http://127.0.0.1:9/A-z.0_9~/?code=c&state=s'],
];

const QUERY = '?code=c&state=s';

/** Every one is a string a URL parser would take, and most resolve off the machine or off the listener. */
const REFUSED_REDIRECTS: [string, unknown][] = [
  ['the loopback hostname', `http://localhost:49152/callback${QUERY}`],
  ['https', `https://127.0.0.1:49152/callback${QUERY}`],
  ['no port', `http://127.0.0.1/callback${QUERY}`],
  ['an empty port', `http://127.0.0.1:/callback${QUERY}`],
  ['port 0', `http://127.0.0.1:0/callback${QUERY}`],
  ['a port with a leading zero', `http://127.0.0.1:08080/callback${QUERY}`],
  ['a port above 65535', `http://127.0.0.1:65536/callback${QUERY}`],
  ['a six-digit port', `http://127.0.0.1:123456/callback${QUERY}`],
  ['userinfo before the address', `http://user@127.0.0.1:49152/callback${QUERY}`],
  ['userinfo with a password', `http://user:pw@127.0.0.1:49152/callback${QUERY}`],
  ['the address as userinfo of another host', `http://127.0.0.1:49152@evil.example/callback${QUERY}`],
  ['a host that starts with the address', `http://127.0.0.1.evil.example:49152/callback${QUERY}`],
  ['another host', `http://evil.example:49152/callback${QUERY}`],
  ['the IPv6 loopback', `http://[::1]:49152/callback${QUERY}`],
  ['another spelling of the address', `http://127.1:49152/callback${QUERY}`],
  ['an upper-case scheme', `HTTP://127.0.0.1:49152/callback${QUERY}`],
  ['no path', 'http://127.0.0.1:49152'],
  ['a query where the path belongs', `http://127.0.0.1:49152${QUERY}`],
  ['a backslash', `http://127.0.0.1:49152\\@evil.example/callback${QUERY}`],
  ['a percent-encoded path', `http://127.0.0.1:49152/%2e%2e/callback${QUERY}`],
  ['a fragment', 'http://127.0.0.1:49152/callback#code=c&state=s'],
  ['a space', `http://127.0.0.1:49152/call back${QUERY}`],
  ['a leading space', ` http://127.0.0.1:49152/callback${QUERY}`],
  ['a trailing newline', `http://127.0.0.1:49152/callback${QUERY}\n`],
  ['no scheme', `//127.0.0.1:49152/callback${QUERY}`],
  ['a path on the Hub', `/callback${QUERY}`],
  ['a script URL', 'javascript:alert(1)'],
  ['an empty string', ''],
];

const WRONG_ANSWERS: [string, unknown][] = [
  ...REFUSED_REDIRECTS.map(([shape, redirect]): [string, unknown] => [
    `a redirect with ${shape}`,
    { redirect },
  ]),
  ['no redirect', {}],
  ['a numeric redirect', { redirect: 49152 }],
  ['a list of redirects', { redirect: [APPROVED] }],
  ['a redirect under another name', { redirect_uri: APPROVED }],
  ['a bare string', APPROVED],
  ['null', null],
];

const CONSENTS = [
  {
    name: 'approvePairing',
    act: () => approvePairing('pair-1', ['read', 'gates']),
    path: APPROVE,
    body: { pairing_id: 'pair-1', scopes: ['read', 'gates'] },
  },
  {
    name: 'denyPairing',
    act: () => denyPairing('pair-1'),
    path: DENY,
    body: { pairing_id: 'pair-1' },
  },
];

afterEach(() => {
  vi.unstubAllGlobals();
  dropSession();
});

describe('startPairing', () => {
  it('posts the request segment as it was given and maps the prompt to its TypeScript shape', async () => {
    const { sent, pending } = stubFetch([post(START, WIRE_PROMPT)]);

    await expect(startPairing(SEGMENT)).resolves.toEqual(PROMPT);

    expect(pending).toEqual([]);
    expect(sent).toEqual([{ path: START, body: { request: SEGMENT }, csrfToken: null }]);
  });

  it('hands back the fingerprint exactly as the Hub rendered it', async () => {
    const rendered = '  sha256:0zco·CORZ / NYy-\tDWpq ';
    stubFetch([post(START, { ...WIRE_PROMPT, fingerprint: rendered })]);

    const prompt = await startPairing(SEGMENT);

    expect(prompt.fingerprint).toBe(rendered);
  });

  it.each([
    ['one scope', ['gates']],
    ['two scopes in the order sent', ['spend', 'read']],
  ])('maps a prompt asking for %s', async (_shape, scopes) => {
    stubFetch([post(START, { ...WIRE_PROMPT, scopes })]);

    await expect(startPairing(SEGMENT)).resolves.toEqual({ ...PROMPT, scopes });
  });

  it.each(WRONG_PROMPTS)('rejects a prompt with %s', async (_shape, body) => {
    const { pending } = stubFetch([post(START, body)]);

    const error = await rejectionOf(startPairing(SEGMENT));

    expect(error.path).toBe(START);
    expect(error.status).toBe(200);
    expect(pending).toEqual([]);
  });

  it('surfaces a Hub refusal with its code', async () => {
    stubFetch([post(START, { error: 'invalid_request' }, 400)]);

    const error = await rejectionOf(startPairing(SEGMENT));

    expect(error.status).toBe(400);
    expect(error.code).toBe('invalid_request');
  });
});

describe('approvePairing', () => {
  it('posts the pairing and the scopes left checked, and resolves to the redirect', async () => {
    const { sent, pending } = stubFetch([post(APPROVE, { redirect: APPROVED })]);

    await expect(approvePairing('pair-1', ['read', 'gates'])).resolves.toBe(APPROVED);

    expect(pending).toEqual([]);
    expect(sent).toEqual([
      { path: APPROVE, body: { pairing_id: 'pair-1', scopes: ['read', 'gates'] }, csrfToken: null },
    ]);
  });
});

describe('denyPairing', () => {
  it('posts the pairing alone, and resolves to the redirect', async () => {
    const { sent, pending } = stubFetch([post(DENY, { redirect: DENIED })]);

    await expect(denyPairing('pair-1')).resolves.toBe(DENIED);

    expect(pending).toEqual([]);
    expect(sent).toEqual([{ path: DENY, body: { pairing_id: 'pair-1' }, csrfToken: null }]);
  });
});

describe.each(CONSENTS)('$name', ({ act, path }) => {
  it.each(LOOPBACK_REDIRECTS)('resolves to a redirect carrying %s, unchanged', async (_shape, redirect) => {
    stubFetch([post(path, { redirect })]);

    await expect(act()).resolves.toBe(redirect);
  });

  it.each(WRONG_ANSWERS)('rejects an answer with %s', async (_shape, body) => {
    const { pending } = stubFetch([post(path, body)]);

    const error = await rejectionOf(act());

    expect(error.path).toBe(path);
    expect(error.status).toBe(200);
    expect(pending).toEqual([]);
  });

  it('surfaces a Hub refusal with its code', async () => {
    stubFetch([post(path, { error: 'pairing_expired' }, 400)]);

    const error = await rejectionOf(act());

    expect(error.status).toBe(400);
    expect(error.code).toBe('pairing_expired');
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
    name: 'startPairing',
    act: (): Promise<unknown> => startPairing(SEGMENT),
    path: START,
    body: { request: SEGMENT },
    answer: WIRE_PROMPT,
    resolved: PROMPT,
  },
  ...CONSENTS.map((consent) => ({ ...consent, answer: { redirect: APPROVED }, resolved: APPROVED })),
])('$name from a tab whose CSRF token went stale', ({ act, path, body, answer, resolved }) => {
  const refused = (code: string) => post(path, { error: code }, 403);

  it.each(['csrf_mismatch', 'csrf_required'])(
    'resends a request refused as %s once, with the same body and the token of the session read',
    async (code) => {
      const { sent, pending, told, stop } = await signedInThen([
        refused(code),
        get('/v1/session', REREAD),
        post(path, answer),
      ]);

      await expect(act()).resolves.toEqual(resolved);

      expect(pending).toEqual([]);
      expect(sent).toEqual([
        { path, body, csrfToken: 'tok-stale' },
        { path: '/v1/session', body: undefined, csrfToken: null },
        { path, body, csrfToken: 'tok-cookie' },
      ]);
      expect(told).not.toHaveBeenCalled();
      stop();
    },
  );

  it('rejects with the second refusal after exactly one resend', async () => {
    const { sent, pending, stop } = await signedInThen([
      refused('csrf_mismatch'),
      get('/v1/session', REREAD),
      refused('csrf_mismatch'),
    ]);

    const error = await rejectionOf(act());

    expect(error.status).toBe(403);
    expect(error.code).toBe('csrf_mismatch');
    expect(pending).toEqual([]);
    expect(sent.map((request) => request.path)).toEqual([path, '/v1/session', path]);
    stop();
  });

  it('resends nothing into a session of another passkey, and tells the listener once', async () => {
    const { sent, pending, told, stop } = await signedInThen([
      refused('csrf_mismatch'),
      get('/v1/session', { csrf_token: 'tok-other', credential_id: 'cred-9' }),
    ]);

    const error = await rejectionOf(act());

    expect(error.path).toBe(path);
    expect(error.code).toBe('csrf_mismatch');
    expect(pending).toEqual([]);
    expect(sent.map((request) => request.path)).toEqual([path, '/v1/session']);
    expect(told).toHaveBeenCalledTimes(1);
    stop();
  });
});

describe('leaveFor', () => {
  it.each(LOOPBACK_REDIRECTS)('sends the browser to a redirect carrying %s, unchanged', (_shape, redirect) => {
    const assign = stubLocation();

    leaveFor(redirect);

    expect(assign.mock.calls).toEqual([[redirect]]);
  });

  it.each(
    REFUSED_REDIRECTS.filter((refused): refused is [string, string] => typeof refused[1] === 'string'),
  )('throws on a redirect with %s, and navigates nowhere', (_shape, redirect) => {
    const assign = stubLocation();

    expect(() => leaveFor(redirect)).toThrow('Not a redirect to the loopback listener');

    expect(assign).not.toHaveBeenCalled();
  });
});
