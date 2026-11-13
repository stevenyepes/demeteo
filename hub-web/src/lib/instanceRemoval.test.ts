import { afterEach, describe, expect, it, vi } from 'vitest';

import { HubApiError, setCsrfToken } from './api';
import { removalProblemText, removeInstance } from './instanceRemoval';
import { WebAuthnCancelledError } from './webauthn';

type Exchange = { method: string; path: string; answer: Response };

/** Every request a double was not told about, held to `[]` after each test. */
let unscripted: string[] = [];

/** Both doubles write here, so a test reads the order across fetch and the browser. */
let order: string[] = [];

/** A request that is not the next one in `script` throws. */
function stubFetch(script: Exchange[]) {
  const pending = [...script];
  const sent: { body: unknown; csrf: string | null }[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const method = init?.method ?? 'GET';
      const next = pending.shift();
      if (next === undefined || next.path !== input || method !== next.method) {
        unscripted.push(`${method} ${String(input)}`);
        throw new Error(`fetch double was not told about ${method} ${String(input)} here`);
      }
      order.push(`${method} ${next.path}`);
      const headers = new Headers(init?.headers);
      sent.push({
        body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined,
        csrf: headers.get('X-CSRF-Token'),
      });
      return next.answer;
    }),
  );
  return { sent, pending };
}

function stubWebAuthn(answer: () => Promise<unknown>) {
  const credentials = {
    create: vi.fn(async () => {
      throw new Error('navigator.credentials.create double was not told to answer');
    }),
    get: vi.fn((_options?: CredentialRequestOptions) => {
      order.push('credentials.get');
      return answer();
    }),
  };
  vi.stubGlobal('navigator', { credentials });
  vi.stubGlobal('PublicKeyCredential', class {});
  return credentials;
}

function exchange(method: string, path: string, body: unknown, status: number): Exchange {
  return {
    method,
    path,
    answer:
      status === 204
        ? new Response(null, { status })
        : new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } }),
  };
}

async function rejectionOf(pending: Promise<unknown>): Promise<unknown> {
  return pending.then(
    (value: unknown) => {
      throw new Error(`expected a rejection, got ${JSON.stringify(value)}`);
    },
    (reason: unknown) => reason,
  );
}

function buffer(...octets: number[]): ArrayBuffer {
  return new Uint8Array(octets).buffer;
}

const INSTANCE_ID = '7f3e2a10-9c4b-4d2e-8f1a-0b6c5d4e3f21';
const INSTANCE_PATH = `/v1/instances/${INSTANCE_ID}`;
const CHALLENGE_PATH = `${INSTANCE_PATH}/remove/challenge`;

const CHALLENGE = {
  ceremony_id: 'cer-4',
  public_key: {
    challenge: '-_-_',
    rpId: 'hub.example',
    allowCredentials: [{ type: 'public-key', id: 'BAU' }],
    userVerification: 'discouraged',
    timeout: 300_000,
  },
};

const ASSERTION = {
  id: 'ignored',
  rawId: buffer(0xfb, 0xff),
  type: 'public-key',
  response: {
    clientDataJSON: buffer(1, 2, 3),
    authenticatorData: buffer(4, 5, 6, 7),
    signature: buffer(8),
    userHandle: null,
  },
};

const ASSERTION_FIELDS = {
  credential_id: '-_8',
  authenticator_data: 'BAUGBw',
  client_data_json: 'AQID',
  signature: 'CA',
};

afterEach(() => {
  vi.unstubAllGlobals();
  setCsrfToken(null);
  order = [];
  const strays = unscripted;
  unscripted = [];
  expect.soft(strays).toEqual([]);
});

describe('removeInstance', () => {
  it('asks for a challenge, signs it with a required-UV passkey, then deletes with the held ceremony', async () => {
    setCsrfToken('csrf-1');
    const { sent, pending } = stubFetch([
      exchange('POST', CHALLENGE_PATH, CHALLENGE, 200),
      exchange('DELETE', INSTANCE_PATH, null, 204),
    ]);
    const credentials = stubWebAuthn(async () => ASSERTION);

    await expect(removeInstance(INSTANCE_ID)).resolves.toBeUndefined();

    expect(order).toEqual([`POST ${CHALLENGE_PATH}`, 'credentials.get', `DELETE ${INSTANCE_PATH}`]);
    expect(credentials.get.mock.calls[0]?.[0]?.publicKey?.userVerification).toBe('required');
    expect(sent).toStrictEqual([
      { body: {}, csrf: 'csrf-1' },
      { body: { ceremony_id: 'cer-4', assertion: ASSERTION_FIELDS }, csrf: 'csrf-1' },
    ]);
    expect(pending).toEqual([]);
  });

  it('asks the Hub nothing for an id that is not an instance id', async () => {
    stubFetch([]);
    stubWebAuthn(async () => ASSERTION);

    const error = await rejectionOf(removeInstance('..'));

    expect(error).toBeInstanceOf(HubApiError);
    expect((error as HubApiError).code).toBe('not_found');
    expect(order).toEqual([]);
  });

  it('sends no DELETE when the passkey prompt is cancelled', async () => {
    stubFetch([exchange('POST', CHALLENGE_PATH, CHALLENGE, 200)]);
    stubWebAuthn(async () => {
      throw new DOMException('cancelled', 'NotAllowedError');
    });

    expect(await rejectionOf(removeInstance(INSTANCE_ID))).toBeInstanceOf(WebAuthnCancelledError);
    expect(order).toEqual([`POST ${CHALLENGE_PATH}`, 'credentials.get']);
  });

  it.each([
    [400, 'ceremony_expired'],
    [403, 'assertion_invalid'],
    [404, 'not_found'],
  ])('surfaces a %i %s refusal once, without resending', async (status, code) => {
    stubFetch([
      exchange('POST', CHALLENGE_PATH, CHALLENGE, 200),
      exchange('DELETE', INSTANCE_PATH, { error: code }, status),
    ]);
    stubWebAuthn(async () => ASSERTION);

    const error = await rejectionOf(removeInstance(INSTANCE_ID));

    expect(error).toBeInstanceOf(HubApiError);
    expect((error as HubApiError).code).toBe(code);
    expect(order.filter((step) => step.startsWith('DELETE'))).toHaveLength(1);
  });

  it('refuses a challenge with no public key before any passkey prompt', async () => {
    stubFetch([exchange('POST', CHALLENGE_PATH, { ceremony_id: 'cer-4' }, 200)]);
    const credentials = stubWebAuthn(async () => ASSERTION);

    expect(await rejectionOf(removeInstance(INSTANCE_ID))).toBeInstanceOf(HubApiError);
    expect(credentials.get).not.toHaveBeenCalled();
  });
});

describe('removalProblemText', () => {
  it.each([
    ['not_found', /already have been removed/],
    ['ceremony_expired', /Start again/],
    ['assertion_invalid', /did not accept that passkey/],
    ['too_many_ceremonies', /Too many passkey prompts/],
  ])('explains %s', (code, text) => {
    expect(removalProblemText(new HubApiError('refused', INSTANCE_PATH, 400, code))).toMatch(text);
  });

  it('adds nothing to a cancelled prompt or an unknown code', () => {
    expect(removalProblemText(new WebAuthnCancelledError())).toBeNull();
    expect(removalProblemText(new HubApiError('refused', INSTANCE_PATH, 500, 'internal'))).toBeNull();
  });
});
