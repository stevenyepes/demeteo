import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  base64UrlToBytes,
  bytesToBase64Url,
  createCredential,
  getAssertion,
  getCredential,
  WebAuthnCancelledError,
  WebAuthnMalformedError,
  WebAuthnUnavailableError,
} from './webauthn';

type Answer = () => Promise<unknown>;

function refusal(method: string): Answer {
  return async () => {
    throw new Error(`navigator.credentials.${method} double was not told to answer`);
  };
}

function credentialsDouble(script: { create?: Answer; get?: Answer }) {
  return {
    create: vi.fn((_options?: CredentialCreationOptions) => (script.create ?? refusal('create'))()),
    get: vi.fn((_options?: CredentialRequestOptions) => (script.get ?? refusal('get'))()),
  };
}

/**
 * Answers only the method in `script` and rejects the other, so a ceremony
 * that reaches the wrong one — or reaches either when it should have stopped
 * first — fails on that instead of resolving against a default credential.
 */
function stubWebAuthn(script: { create?: Answer; get?: Answer } = {}) {
  const credentials = credentialsDouble(script);
  vi.stubGlobal('navigator', { credentials });
  vi.stubGlobal('PublicKeyCredential', class {});
  return credentials;
}

function buffer(...octets: number[]): ArrayBuffer {
  return new Uint8Array(octets).buffer;
}

function octetsOf(value: unknown): number[] {
  if (!(value instanceof ArrayBuffer)) throw new Error(`expected an ArrayBuffer, got ${String(value)}`);
  return Array.from(new Uint8Array(value));
}

async function rejectionOf(pending: Promise<unknown>): Promise<unknown> {
  return pending.then(
    (value: unknown) => {
      throw new Error(`expected a rejection, got ${JSON.stringify(value)}`);
    },
    (reason: unknown) => reason,
  );
}

const CREATION_OPTIONS = {
  publicKey: {
    rp: { id: 'hub.example', name: 'Demeteo Hub' },
    user: { id: 'AQID', name: 'laptop', displayName: 'laptop' },
    challenge: '-_-_',
    pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
    timeout: 300_000,
    excludeCredentials: [{ type: 'public-key', id: 'BAU' }, { type: 'public-key', id: 'Bg' }],
    authenticatorSelection: { residentKey: 'required', userVerification: 'required' },
  },
};

const REQUEST_OPTIONS = {
  publicKey: {
    challenge: '-_-_',
    rpId: 'hub.example',
    allowCredentials: [{ type: 'public-key', id: 'BAU' }, { type: 'public-key', id: 'Bg' }],
    userVerification: 'required',
    timeout: 300_000,
  },
};

const ATTESTATION = {
  id: 'ignored',
  rawId: buffer(0xfb, 0xff),
  type: 'public-key',
  response: { clientDataJSON: buffer(1, 2, 3), attestationObject: buffer(4, 5, 6, 7) },
};

const ASSERTION = {
  id: 'ignored',
  rawId: buffer(0xfb, 0xff),
  type: 'public-key',
  response: {
    clientDataJSON: buffer(1, 2, 3),
    authenticatorData: buffer(4, 5, 6, 7),
    signature: buffer(8),
    userHandle: buffer(9, 10),
  },
};

const CEREMONIES = [
  { name: 'createCredential', method: 'create', run: () => createCredential(CREATION_OPTIONS) },
  { name: 'getCredential', method: 'get', run: () => getCredential(REQUEST_OPTIONS) },
  { name: 'getAssertion', method: 'get', run: () => getAssertion(REQUEST_OPTIONS) },
] as const;

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('base64url', () => {
  it.each([
    { octets: [], text: '' },
    { octets: [0xfb], text: '-w' },
    { octets: [0xfb, 0xff], text: '-_8' },
    { octets: [0xfb, 0xff, 0xbf], text: '-_-_' },
    { octets: [0, 1, 2, 3], text: 'AAECAw' },
  ])('round-trips $octets as "$text"', ({ octets, text }) => {
    expect(bytesToBase64Url(buffer(...octets))).toBe(text);
    expect(Array.from(base64UrlToBytes(text))).toEqual(octets);
  });

  it('encodes only the window a view covers', () => {
    expect(bytesToBase64Url(new Uint8Array([9, 0xfb, 9]).subarray(1, 2))).toBe('-w');
  });

  it.each(['A', 'AAAAA', '-w==', 'A+/A', 'AA A'])('refuses to decode "%s"', (text) => {
    expect(() => base64UrlToBytes(text)).toThrow(WebAuthnMalformedError);
  });
});

describe('createCredential', () => {
  it('hands create() the options with ArrayBuffer binary members', async () => {
    const credentials = stubWebAuthn({ create: async () => ATTESTATION });

    await createCredential(CREATION_OPTIONS);

    expect(credentials.create).toHaveBeenCalledTimes(1);
    const publicKey = credentials.create.mock.calls[0]?.[0]?.publicKey;
    expect(octetsOf(publicKey?.challenge)).toEqual([0xfb, 0xff, 0xbf]);
    expect(octetsOf(publicKey?.user.id)).toEqual([1, 2, 3]);
    expect(publicKey?.excludeCredentials?.map((descriptor) => octetsOf(descriptor.id))).toEqual([[4, 5], [6]]);
    expect(publicKey).toMatchObject({
      rp: { id: 'hub.example', name: 'Demeteo Hub' },
      user: { name: 'laptop', displayName: 'laptop' },
      pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
      timeout: 300_000,
      excludeCredentials: [{ type: 'public-key' }, { type: 'public-key' }],
      authenticatorSelection: { residentKey: 'required', userVerification: 'required' },
    });
  });

  it('accepts options that exclude no credential', async () => {
    const credentials = stubWebAuthn({ create: async () => ATTESTATION });
    const { excludeCredentials: _, ...publicKey } = CREATION_OPTIONS.publicKey;

    await createCredential({ publicKey });

    expect(credentials.create.mock.calls[0]?.[0]?.publicKey?.excludeCredentials).toBeUndefined();
  });

  it('returns the registration credential as JSON', async () => {
    stubWebAuthn({ create: async () => ATTESTATION });

    await expect(createCredential(CREATION_OPTIONS)).resolves.toStrictEqual({
      id: '-_8',
      rawId: '-_8',
      type: 'public-key',
      response: { clientDataJSON: 'AQID', attestationObject: 'BAUGBw' },
      clientExtensionResults: {},
    });
  });

  it.each([
    ['no publicKey', {}],
    ['a challenge that is not base64url', { publicKey: { ...CREATION_OPTIONS.publicKey, challenge: 'A+/A' } }],
    ['a challenge that is not a string', { publicKey: { ...CREATION_OPTIONS.publicKey, challenge: 7 } }],
    ['no user', { publicKey: { ...CREATION_OPTIONS.publicKey, user: undefined } }],
    ['a user id that is not base64url', { publicKey: { ...CREATION_OPTIONS.publicKey, user: { id: 'A' } } }],
    ['excludeCredentials that is not a list', { publicKey: { ...CREATION_OPTIONS.publicKey, excludeCredentials: 'BAU' } }],
    ['an excluded id that is not base64url', { publicKey: { ...CREATION_OPTIONS.publicKey, excludeCredentials: [{ id: 4 }] } }],
  ])('refuses options with %s before reaching the authenticator', async (_case, options) => {
    const credentials = stubWebAuthn();

    const error = await rejectionOf(createCredential(options));

    expect(error).toBeInstanceOf(WebAuthnMalformedError);
    expect(error).toHaveProperty('message', 'The Hub sent unusable creation options');
    expect(credentials.create).not.toHaveBeenCalled();
  });

  it.each([
    ['rawId', { ...ATTESTATION, rawId: '-_8' }],
    ['response', { ...ATTESTATION, response: undefined }],
    ['clientDataJSON', { ...ATTESTATION, response: { ...ATTESTATION.response, clientDataJSON: null } }],
    ['attestationObject', { ...ATTESTATION, response: { clientDataJSON: buffer(1) } }],
  ])('refuses a credential without a binary %s', async (_member, credential) => {
    stubWebAuthn({ create: async () => credential });

    expect(await rejectionOf(createCredential(CREATION_OPTIONS))).toBeInstanceOf(WebAuthnMalformedError);
  });
});

describe('getCredential', () => {
  it('hands get() the options with ArrayBuffer binary members', async () => {
    const credentials = stubWebAuthn({ get: async () => ASSERTION });

    await getCredential(REQUEST_OPTIONS);

    expect(credentials.get).toHaveBeenCalledTimes(1);
    const publicKey = credentials.get.mock.calls[0]?.[0]?.publicKey;
    expect(octetsOf(publicKey?.challenge)).toEqual([0xfb, 0xff, 0xbf]);
    expect(publicKey?.allowCredentials?.map((descriptor) => octetsOf(descriptor.id))).toEqual([[4, 5], [6]]);
    expect(publicKey).toMatchObject({
      rpId: 'hub.example',
      allowCredentials: [{ type: 'public-key' }, { type: 'public-key' }],
      userVerification: 'required',
      timeout: 300_000,
    });
  });

  it('accepts options that allow any credential', async () => {
    const credentials = stubWebAuthn({ get: async () => ASSERTION });
    const { allowCredentials: _, ...publicKey } = REQUEST_OPTIONS.publicKey;

    await getCredential({ publicKey });

    expect(credentials.get.mock.calls[0]?.[0]?.publicKey?.allowCredentials).toBeUndefined();
  });

  it('returns the sign-in credential as JSON', async () => {
    stubWebAuthn({ get: async () => ASSERTION });

    await expect(getCredential(REQUEST_OPTIONS)).resolves.toStrictEqual({
      id: '-_8',
      rawId: '-_8',
      type: 'public-key',
      response: { clientDataJSON: 'AQID', authenticatorData: 'BAUGBw', signature: 'CA', userHandle: 'CQo' },
      clientExtensionResults: {},
    });
  });

  it('reports a user handle the authenticator did not return as null', async () => {
    stubWebAuthn({ get: async () => ({ ...ASSERTION, response: { ...ASSERTION.response, userHandle: null } }) });

    const credential = await getCredential(REQUEST_OPTIONS);

    expect(credential.response.userHandle).toBeNull();
  });

  it.each([
    ['no publicKey', { challenge: '-_-_' }],
    ['a challenge that is not base64url', { publicKey: { ...REQUEST_OPTIONS.publicKey, challenge: '-_-_=' } }],
    ['allowCredentials that is not a list', { publicKey: { ...REQUEST_OPTIONS.publicKey, allowCredentials: {} } }],
    ['an allowed id that is not base64url', { publicKey: { ...REQUEST_OPTIONS.publicKey, allowCredentials: [{}] } }],
  ])('refuses options with %s before reaching the authenticator', async (_case, options) => {
    const credentials = stubWebAuthn();

    const error = await rejectionOf(getCredential(options));

    expect(error).toBeInstanceOf(WebAuthnMalformedError);
    expect(error).toHaveProperty('message', 'The Hub sent unusable request options');
    expect(credentials.get).not.toHaveBeenCalled();
  });

  it.each(['clientDataJSON', 'authenticatorData', 'signature'])(
    'refuses a credential without a binary %s',
    async (member) => {
      stubWebAuthn({ get: async () => ({ ...ASSERTION, response: { ...ASSERTION.response, [member]: undefined } }) });

      expect(await rejectionOf(getCredential(REQUEST_OPTIONS))).toBeInstanceOf(WebAuthnMalformedError);
    },
  );
});

describe('getAssertion', () => {
  it('returns the four assertion fields, the credential id taken from rawId', async () => {
    const credentials = stubWebAuthn({ get: async () => ASSERTION });

    await expect(getAssertion(REQUEST_OPTIONS)).resolves.toStrictEqual({
      credential_id: '-_8',
      authenticator_data: 'BAUGBw',
      client_data_json: 'AQID',
      signature: 'CA',
    });
    expect(octetsOf(credentials.get.mock.calls[0]?.[0]?.publicKey?.challenge)).toEqual([0xfb, 0xff, 0xbf]);
  });
});

describe.each(CEREMONIES)('$name', ({ method, run }) => {
  it('reports WebAuthn as unavailable where navigator.credentials is missing', async () => {
    vi.stubGlobal('navigator', {});
    vi.stubGlobal('PublicKeyCredential', class {});

    expect(await rejectionOf(run())).toBeInstanceOf(WebAuthnUnavailableError);
  });

  it('reports WebAuthn as unavailable where PublicKeyCredential is missing', async () => {
    const credentials = credentialsDouble({});
    vi.stubGlobal('navigator', { credentials });

    expect(await rejectionOf(run())).toBeInstanceOf(WebAuthnUnavailableError);
    expect(credentials[method]).not.toHaveBeenCalled();
  });

  it('reports a NotAllowedError as a cancel', async () => {
    stubWebAuthn({
      [method]: async () => {
        throw new DOMException('The operation either timed out or was not allowed.', 'NotAllowedError');
      },
    });

    expect(await rejectionOf(run())).toBeInstanceOf(WebAuthnCancelledError);
  });

  it('reports a null credential as a cancel', async () => {
    stubWebAuthn({ [method]: async () => null });

    expect(await rejectionOf(run())).toBeInstanceOf(WebAuthnCancelledError);
  });

  it('lets any other authenticator failure through unchanged', async () => {
    const failure = new DOMException('The authenticator is already registered.', 'InvalidStateError');
    stubWebAuthn({
      [method]: async () => {
        throw failure;
      },
    });

    expect(await rejectionOf(run())).toBe(failure);
  });
});
