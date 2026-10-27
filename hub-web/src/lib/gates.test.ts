import { afterEach, describe, expect, it, vi } from 'vitest';

import { HubApiError } from './api';
import { decideGate, type GateDecision } from './gates';
import { WebAuthnCancelledError } from './webauthn';

type Exchange = { path: string; answer: Response };

/** Every request a double was not told about, held to `[]` after each test. */
let unscripted: string[] = [];

/** Both doubles write here, so a test reads the order across fetch and the browser. */
let order: string[] = [];

/** The scripted double of `requests.test.ts`, POST-only: a request that is not the next one in `script` throws. */
function stubFetch(script: Exchange[]) {
  const pending = [...script];
  const bodies: unknown[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const method = init?.method ?? 'GET';
      const next = pending.shift();
      if (next === undefined || next.path !== input || method !== 'POST') {
        unscripted.push(`${method} ${String(input)}`);
        throw new Error(`fetch double was not told about ${method} ${String(input)} here`);
      }
      order.push(`POST ${next.path}`);
      if (typeof init?.body === 'string') bodies.push(JSON.parse(init.body));
      return next.answer;
    }),
  );
  return { bodies, pending };
}

/** Answers `get` with `answer` and refuses `create`, which no gate decision reaches. */
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

function post(path: string, body: unknown, status: number): Exchange {
  return {
    path,
    answer: new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } }),
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
const CHALLENGE_PATH = `/v1/instances/${INSTANCE_ID}/gates/challenge`;
const DECISION_PATH = `/v1/instances/${INSTANCE_ID}/gates/decision`;
const EXPIRES_AT = 1_700_000_060;

/** The Hub asks for less than the gate needs; hub-web must not take it at its word. */
const CHALLENGE = {
  ceremony_id: 'cer-9',
  request_id: 'q7Zr-1_x',
  expires_at: EXPIRES_AT,
  gate_challenge: 'AAECAw',
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

const TICKET = { request_id: CHALLENGE.request_id, expires_at: EXPIRES_AT };

function decide(decision: GateDecision, feedback?: string) {
  return decideGate({ instanceId: INSTANCE_ID, featureId: 'f-1', stepExecutionId: 'se-3', decision, feedback });
}

afterEach(() => {
  vi.unstubAllGlobals();
  order = [];
  const strays = unscripted;
  unscripted = [];
  // Soft: a throw here would skip the setup file's own `afterEach`.
  expect.soft(strays).toEqual([]);
});

describe('decideGate', () => {
  it.each(['approve', 'cancel'] as const)('%s: challenge, then a required-UV passkey, then the decision', async (decision) => {
    const { bodies, pending } = stubFetch([post(CHALLENGE_PATH, CHALLENGE, 200), post(DECISION_PATH, TICKET, 202)]);
    const credentials = stubWebAuthn(async () => ASSERTION);

    await expect(decide(decision)).resolves.toEqual({ requestId: CHALLENGE.request_id, expiresAt: EXPIRES_AT });

    expect(order).toEqual([`POST ${CHALLENGE_PATH}`, 'credentials.get', `POST ${DECISION_PATH}`]);
    expect(credentials.get).toHaveBeenCalledTimes(1);
    expect(credentials.get.mock.calls[0]?.[0]?.publicKey?.userVerification).toBe('required');
    expect(bodies).toStrictEqual([
      { feature_id: 'f-1', step_execution_id: 'se-3', decision },
      { ceremony_id: 'cer-9', request_id: 'q7Zr-1_x', assertion: ASSERTION_FIELDS },
    ]);
    expect(pending).toEqual([]);
  });

  it('sends feedback beside the assertion, as written', async () => {
    const { bodies } = stubFetch([post(CHALLENGE_PATH, CHALLENGE, 200), post(DECISION_PATH, TICKET, 202)]);
    stubWebAuthn(async () => ASSERTION);

    await decide('cancel', ' wrong branch ');

    expect(bodies[1]).toStrictEqual({
      ceremony_id: 'cer-9',
      request_id: 'q7Zr-1_x',
      assertion: ASSERTION_FIELDS,
      feedback: ' wrong branch ',
    });
  });

  it.each(['', '  \n'])('omits feedback %j', async (feedback) => {
    const { bodies } = stubFetch([post(CHALLENGE_PATH, CHALLENGE, 200), post(DECISION_PATH, TICKET, 202)]);
    stubWebAuthn(async () => ASSERTION);

    await decide('approve', feedback);

    expect(Object.keys(bodies[1] as object)).toEqual(['ceremony_id', 'request_id', 'assertion']);
  });

  it('sends no decision when the passkey prompt is cancelled', async () => {
    const { pending } = stubFetch([post(CHALLENGE_PATH, CHALLENGE, 200)]);
    stubWebAuthn(async () => {
      throw new DOMException('dismissed', 'NotAllowedError');
    });

    expect(await rejectionOf(decide('approve'))).toBeInstanceOf(WebAuthnCancelledError);
    expect(order).toEqual([`POST ${CHALLENGE_PATH}`, 'credentials.get']);
    expect(pending).toEqual([]);
  });

  it.each([
    'ceremony_expired',
    'challenge_mismatch',
  ])('surfaces %s from the decision and does not resubmit', async (code) => {
    stubFetch([post(CHALLENGE_PATH, CHALLENGE, 200), post(DECISION_PATH, { error: code }, 400)]);
    const credentials = stubWebAuthn(async () => ASSERTION);

    const error = await rejectionOf(decide('approve'));

    expect(error).toBeInstanceOf(HubApiError);
    expect(error).toMatchObject({ status: 400, code, path: DECISION_PATH });
    expect(order).toEqual([`POST ${CHALLENGE_PATH}`, 'credentials.get', `POST ${DECISION_PATH}`]);
    expect(credentials.get).toHaveBeenCalledTimes(1);
  });

  it('surfaces too_many_ceremonies from the challenge without asking the passkey', async () => {
    stubFetch([post(CHALLENGE_PATH, { error: 'too_many_ceremonies' }, 429)]);
    const credentials = stubWebAuthn(async () => ASSERTION);

    const error = await rejectionOf(decide('cancel'));

    expect(error).toMatchObject({ name: 'HubApiError', code: 'too_many_ceremonies' });
    expect(credentials.get).not.toHaveBeenCalled();
  });

  it('refuses a challenge without its public key before asking the passkey', async () => {
    const { public_key: _, ...withoutPublicKey } = CHALLENGE;
    stubFetch([post(CHALLENGE_PATH, withoutPublicKey, 200)]);
    const credentials = stubWebAuthn(async () => ASSERTION);

    const error = await rejectionOf(decide('approve'));

    expect(error).toBeInstanceOf(HubApiError);
    expect(credentials.get).not.toHaveBeenCalled();
  });

  it('answers an id that is not a UUID as not found, without asking', async () => {
    stubFetch([]);
    const credentials = stubWebAuthn(async () => ASSERTION);

    const error = await rejectionOf(
      decideGate({ instanceId: '../fleet', featureId: 'f-1', stepExecutionId: 'se-3', decision: 'approve' }),
    );

    expect(error).toMatchObject({ name: 'HubApiError', status: 404, code: 'not_found' });
    expect(order).toEqual([]);
    expect(credentials.get).not.toHaveBeenCalled();
  });
});
