import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { dropSession, HubApiError } from './api';
import { approveDevice, denyDevice, lookUpDevice } from './devicePairing';

const webauthn = vi.hoisted(() => ({
  getAssertion: vi.fn<(options: unknown) => Promise<unknown>>(),
}));

vi.mock('./webauthn', () => webauthn);

type Exchange = { path: string; answer: Response };

type Sent = { path: string; body: unknown };

/** The scripted double of `pairing.test.ts`: a request that is not the next one in `script` throws. */
function stubFetch(script: Exchange[]) {
  const pending = [...script];
  const sent: Sent[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const next = pending.shift();
      if (next === undefined || next.path !== input || init?.method !== 'POST') {
        throw new Error(`fetch double was not told about ${init?.method ?? 'GET'} ${String(input)} here`);
      }
      sent.push({
        path: next.path,
        body: typeof init?.body === 'string' ? (JSON.parse(init.body) as unknown) : undefined,
      });
      return next.answer;
    }),
  );
  return { sent, pending };
}

function post(path: string, body: unknown, status = 200): Exchange {
  return {
    path,
    answer: new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } }),
  };
}

async function rejectionOf(pending: Promise<unknown>): Promise<unknown> {
  return pending.then(
    () => {
      throw new Error('expected a rejection');
    },
    (reason: unknown) => reason,
  );
}

const LOOKUP = '/v1/pairing/device/lookup';
const CHALLENGE = '/v1/pairing/device/challenge';
const APPROVE = '/v1/pairing/device/approve';
const DENY = '/v1/pairing/device/deny';

const WIRE_PROMPT = {
  user_code: 'BCDF-GHJK',
  fingerprint: 'SHA256:0ZcO CORZ NYy- DWpq',
  instance_name: 'build-01',
  os: 'linux',
  arch: 'x86_64',
  app_version: '1.4.0',
  scopes: ['read', 'gates'],
  expires_at: 1_700_000_600,
};

const OPTIONS = { publicKey: { challenge: 'Y2hhbGxlbmdl', allowCredentials: [] } };
const ASSERTION = {
  credential_id: 'Y3JlZA',
  authenticator_data: 'YXV0aA',
  client_data_json: 'Y2xpZW50',
  signature: 'c2ln',
};

beforeEach(() => {
  webauthn.getAssertion.mockReset().mockRejectedValue(new Error('getAssertion was not scripted'));
});

afterEach(() => {
  vi.unstubAllGlobals();
  dropSession();
});

describe('lookUpDevice', () => {
  it('posts the code as typed and maps the prompt to its TypeScript shape', async () => {
    const { sent, pending } = stubFetch([post(LOOKUP, WIRE_PROMPT)]);

    await expect(lookUpDevice('bcdfghjk')).resolves.toEqual({
      userCode: 'BCDF-GHJK',
      fingerprint: 'SHA256:0ZcO CORZ NYy- DWpq',
      instanceName: 'build-01',
      os: 'linux',
      arch: 'x86_64',
      appVersion: '1.4.0',
      scopes: ['read', 'gates'],
      expiresAt: 1_700_000_600,
    });

    expect(pending).toEqual([]);
    expect(sent).toEqual([{ path: LOOKUP, body: { user_code: 'bcdfghjk' } }]);
  });

  it.each<[string, unknown]>([
    ['no user_code', { ...WIRE_PROMPT, user_code: undefined }],
    ['no fingerprint', { ...WIRE_PROMPT, fingerprint: undefined }],
    ['an unknown scope', { ...WIRE_PROMPT, scopes: ['read', 'admin'] }],
    ['a string expires_at', { ...WIRE_PROMPT, expires_at: '1700000600' }],
    ['a desktop prompt', { ...WIRE_PROMPT, user_code: undefined, pairing_id: 'pair-1' }],
  ])('rejects a prompt with %s', async (_shape, body) => {
    stubFetch([post(LOOKUP, body)]);

    const error = await rejectionOf(lookUpDevice('BCDF-GHJK'));

    expect(error).toBeInstanceOf(HubApiError);
    expect((error as HubApiError).status).toBe(200);
  });

  it('surfaces the miss bound with its code', async () => {
    stubFetch([post(LOOKUP, { error: 'too_many_lookups' }, 429)]);

    const error = await rejectionOf(lookUpDevice('BCDF-GHJK'));

    expect(error).toBeInstanceOf(HubApiError);
    expect((error as HubApiError).code).toBe('too_many_lookups');
  });
});

describe('approveDevice', () => {
  it('asserts over the challenge the Hub issued and sends that assertion untouched', async () => {
    const { sent, pending } = stubFetch([
      post(CHALLENGE, { ceremony_id: 'cer-1', options: OPTIONS }),
      post(APPROVE, { answer: 'approved' }),
    ]);
    webauthn.getAssertion.mockResolvedValueOnce(ASSERTION);

    await approveDevice('BCDF-GHJK', ['gates']);

    expect(pending).toEqual([]);
    expect(webauthn.getAssertion.mock.calls).toEqual([[OPTIONS]]);
    expect(sent).toEqual([
      { path: CHALLENGE, body: { user_code: 'BCDF-GHJK', scopes: ['gates'] } },
      { path: APPROVE, body: { ceremony_id: 'cer-1', assertion: ASSERTION } },
    ]);
  });

  it('sends nothing to approve when the passkey prompt fails', async () => {
    const { pending } = stubFetch([
      post(CHALLENGE, { ceremony_id: 'cer-1', options: OPTIONS }),
      post(APPROVE, { answer: 'approved' }),
    ]);
    const cancelled = new Error('cancelled');
    webauthn.getAssertion.mockRejectedValueOnce(cancelled);

    await expect(approveDevice('BCDF-GHJK', ['read'])).rejects.toBe(cancelled);

    expect(pending.map((exchange) => exchange.path)).toEqual([APPROVE]);
  });

  it.each<[string, unknown]>([
    ['no ceremony id', { options: OPTIONS }],
    ['no options', { ceremony_id: 'cer-1' }],
  ])('asks no passkey over a challenge with %s', async (_shape, body) => {
    stubFetch([post(CHALLENGE, body)]);

    await expect(approveDevice('BCDF-GHJK', ['read'])).rejects.toBeInstanceOf(HubApiError);

    expect(webauthn.getAssertion).not.toHaveBeenCalled();
  });

  it('is not satisfied by an answer other than approved', async () => {
    stubFetch([post(CHALLENGE, { ceremony_id: 'cer-1', options: OPTIONS }), post(APPROVE, { answer: 'denied' })]);
    webauthn.getAssertion.mockResolvedValueOnce(ASSERTION);

    await expect(approveDevice('BCDF-GHJK', ['read'])).rejects.toBeInstanceOf(HubApiError);
  });
});

describe('denyDevice', () => {
  it('posts the code and asks for no passkey', async () => {
    const { sent } = stubFetch([post(DENY, { answer: 'denied' })]);

    await denyDevice('BCDF-GHJK');

    expect(sent).toEqual([{ path: DENY, body: { user_code: 'BCDF-GHJK' } }]);
    expect(webauthn.getAssertion).not.toHaveBeenCalled();
  });
});
