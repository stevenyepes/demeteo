/**
 * hub-web's whole WebAuthn surface, and the only module that names
 * `navigator.credentials`. jsdom has neither it nor `PublicKeyCredential`, so
 * a view that called the browser directly could not be rendered in a test;
 * views call these functions and their tests mock this one module.
 *
 * The Hub speaks JSON with every binary member as base64url without padding,
 * and the browser speaks `ArrayBuffer`. Both conversions are done by hand
 * rather than through `PublicKeyCredential.parseCreationOptionsFromJSON` /
 * `parseRequestOptionsFromJSON` / `toJSON`, which not every browser that has
 * passkeys has. For the same reason a credential is read by its members and
 * never by `instanceof`: whatever answers `navigator.credentials` — an
 * extension wrapping it included — need not hand back a real
 * `PublicKeyCredential`.
 */

type CredentialJson<Response> = {
  id: string;
  rawId: string;
  type: 'public-key';
  response: Response;
  clientExtensionResults: Record<string, never>;
};

/**
 * The `credential` member of the Hub's registration and add-passkey `finish`
 * bodies: the `PublicKeyCredential.toJSON()` shape of an attestation. The Hub
 * deserializes exactly these members, so a change here is a change there.
 */
export type RegistrationCredential = CredentialJson<{
  clientDataJSON: string;
  attestationObject: string;
}>;

/**
 * The `credential` member of the Hub's sign-in `finish` body. `userHandle` is
 * always present and is `null` when the authenticator returned none, where
 * `toJSON()` would omit the member.
 */
export type SignInCredential = CredentialJson<{
  clientDataJSON: string;
  authenticatorData: string;
  signature: string;
  userHandle: string | null;
}>;

/** The four fields of the protocol's `WebAuthnAssertion`, each base64url. */
export type AssertionFields = {
  credential_id: string;
  authenticator_data: string;
  client_data_json: string;
  signature: string;
};

/** No `navigator.credentials` or no `PublicKeyCredential`: an insecure context, or a browser without passkeys. */
export class WebAuthnUnavailableError extends Error {
  constructor() {
    super('This browser does not offer WebAuthn here');
    this.name = 'WebAuthnUnavailableError';
  }
}

/**
 * The ceremony ended without a credential. The browser reports a dismissed
 * prompt, a timeout and an authenticator that refused all as `NotAllowedError`
 * so that a page cannot tell them apart, which is why this is one error.
 */
export class WebAuthnCancelledError extends Error {
  constructor() {
    super('The passkey prompt was cancelled or the authenticator refused');
    this.name = 'WebAuthnCancelledError';
  }
}

/** Options from the Hub, or a credential from the browser, that are not in the shape this module converts. */
export class WebAuthnMalformedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'WebAuthnMalformedError';
  }
}

const BASE64URL = /^[A-Za-z0-9_-]*$/;

export function bytesToBase64Url(bytes: ArrayBuffer | ArrayBufferView): string {
  const octets = ArrayBuffer.isView(bytes)
    ? new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength)
    : new Uint8Array(bytes);
  let binary = '';
  for (const octet of octets) binary += String.fromCharCode(octet);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function base64UrlToBytes(text: string): Uint8Array<ArrayBuffer> {
  if (!isBase64Url(text)) throw new WebAuthnMalformedError('Not base64url without padding');
  const binary = atob(text.replace(/-/g, '+').replace(/_/g, '/'));
  const octets = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) octets[index] = binary.charCodeAt(index);
  return octets;
}

/** Runs a registration ceremony over the `options` of a Hub `…/start` reply. */
export async function createCredential(options: unknown): Promise<RegistrationCredential> {
  const credentials = credentialsContainer();
  if (!isCreationOptions(options)) throw new WebAuthnMalformedError('The Hub sent unusable creation options');
  const publicKey = creationArgument(options.publicKey);

  const credential = await ceremony(() => credentials.create({ publicKey }));

  const response = memberOf(credential, 'response');
  return {
    ...identityOf(credential),
    response: {
      clientDataJSON: encoded(response, 'clientDataJSON'),
      attestationObject: encoded(response, 'attestationObject'),
    },
    clientExtensionResults: {},
  };
}

/** Runs a sign-in ceremony over the `options` of `POST /v1/session/start`. */
export async function getCredential(options: unknown): Promise<SignInCredential> {
  const credential = await assertionCeremony(options);
  const response = memberOf(credential, 'response');
  const userHandle = memberOf(response, 'userHandle');
  return {
    ...identityOf(credential),
    response: {
      clientDataJSON: encoded(response, 'clientDataJSON'),
      authenticatorData: encoded(response, 'authenticatorData'),
      signature: encoded(response, 'signature'),
      userHandle: isBytes(userHandle) ? bytesToBase64Url(userHandle) : null,
    },
    clientExtensionResults: {},
  };
}

/** Asserts over the Hub-chosen challenge of an endorsement or a revocation. */
export async function getAssertion(options: unknown): Promise<AssertionFields> {
  return assertionFieldsOf(await assertionCeremony(options));
}

/**
 * Asserts over the `public_key` of a gate challenge, which the Hub sends bare
 * rather than wrapped as `{ publicKey }`. User verification is required here
 * whatever the Hub asked for: the Hub checks only an assertion's shape, and
 * the desktop's check of the user-verified flag (docs/HUB.md §8) is not built
 * (§2), so the Hub's options must not be able to ask for less.
 */
export async function getGateAssertion(publicKey: unknown): Promise<AssertionFields> {
  const credential = await assertionCeremony({ publicKey }, (request) => ({
    ...request,
    userVerification: 'required',
  }));
  return assertionFieldsOf(credential);
}

type DescriptorJson = Omit<PublicKeyCredentialDescriptor, 'id'> & { id: string };

type CreationOptionsJson = Omit<
  PublicKeyCredentialCreationOptions,
  'challenge' | 'user' | 'excludeCredentials'
> & {
  challenge: string;
  user: Omit<PublicKeyCredentialUserEntity, 'id'> & { id: string };
  excludeCredentials?: DescriptorJson[];
};

type RequestOptionsJson = Omit<PublicKeyCredentialRequestOptions, 'challenge' | 'allowCredentials'> & {
  challenge: string;
  allowCredentials?: DescriptorJson[];
};

/**
 * This guard and the request one check the binary members this module decodes
 * and nothing else. Every other member is handed to the browser as received,
 * and `create()` / `get()` reject a missing or mistyped one with a `TypeError`
 * of their own — a second copy of that validation here would only drift from
 * the browser's.
 */
function isCreationOptions(value: unknown): value is { publicKey: CreationOptionsJson } {
  const publicKey = memberOf(value, 'publicKey');
  return (
    isBase64Url(memberOf(publicKey, 'challenge')) &&
    isBase64Url(memberOf(memberOf(publicKey, 'user'), 'id')) &&
    isDescriptorList(memberOf(publicKey, 'excludeCredentials'))
  );
}

function isRequestOptions(value: unknown): value is { publicKey: RequestOptionsJson } {
  const publicKey = memberOf(value, 'publicKey');
  return (
    isBase64Url(memberOf(publicKey, 'challenge')) && isDescriptorList(memberOf(publicKey, 'allowCredentials'))
  );
}

function isDescriptorList(value: unknown): value is DescriptorJson[] | undefined {
  if (value === undefined) return true;
  return Array.isArray(value) && value.every((descriptor) => isBase64Url(memberOf(descriptor, 'id')));
}

function creationArgument(json: CreationOptionsJson): PublicKeyCredentialCreationOptions {
  return {
    ...json,
    challenge: base64UrlToBytes(json.challenge).buffer,
    user: { ...json.user, id: base64UrlToBytes(json.user.id).buffer },
    excludeCredentials: json.excludeCredentials?.map(descriptorArgument),
  };
}

function requestArgument(json: RequestOptionsJson): PublicKeyCredentialRequestOptions {
  return {
    ...json,
    challenge: base64UrlToBytes(json.challenge).buffer,
    allowCredentials: json.allowCredentials?.map(descriptorArgument),
  };
}

function descriptorArgument(json: DescriptorJson): PublicKeyCredentialDescriptor {
  return { ...json, id: base64UrlToBytes(json.id).buffer };
}

async function assertionCeremony(
  options: unknown,
  amend: (request: PublicKeyCredentialRequestOptions) => PublicKeyCredentialRequestOptions = (request) => request,
): Promise<Credential> {
  const credentials = credentialsContainer();
  if (!isRequestOptions(options)) throw new WebAuthnMalformedError('The Hub sent unusable request options');
  const publicKey = amend(requestArgument(options.publicKey));
  return ceremony(() => credentials.get({ publicKey }));
}

function assertionFieldsOf(credential: Credential): AssertionFields {
  const response = memberOf(credential, 'response');
  return {
    credential_id: encoded(credential, 'rawId'),
    authenticator_data: encoded(response, 'authenticatorData'),
    client_data_json: encoded(response, 'clientDataJSON'),
    signature: encoded(response, 'signature'),
  };
}

function credentialsContainer(): CredentialsContainer {
  const credentials: unknown = typeof navigator === 'undefined' ? undefined : memberOf(navigator, 'credentials');
  if (!isCredentialsContainer(credentials) || typeof PublicKeyCredential === 'undefined') {
    throw new WebAuthnUnavailableError();
  }
  return credentials;
}

function isCredentialsContainer(value: unknown): value is CredentialsContainer {
  return typeof memberOf(value, 'create') === 'function' && typeof memberOf(value, 'get') === 'function';
}

async function ceremony(run: () => Promise<Credential | null>): Promise<Credential> {
  let credential: Credential | null;
  try {
    credential = await run();
  } catch (error) {
    if (memberOf(error, 'name') === 'NotAllowedError') throw new WebAuthnCancelledError();
    throw error;
  }
  if (credential === null) throw new WebAuthnCancelledError();
  return credential;
}

/** `id` is derived from `rawId` rather than read, so the two cannot disagree. */
function identityOf(credential: Credential): { id: string; rawId: string; type: 'public-key' } {
  const rawId = encoded(credential, 'rawId');
  return { id: rawId, rawId, type: 'public-key' };
}

function encoded(source: unknown, member: string): string {
  const bytes = memberOf(source, member);
  if (!isBytes(bytes)) throw new WebAuthnMalformedError(`The browser returned a credential without ${member}`);
  return bytesToBase64Url(bytes);
}

function isBytes(value: unknown): value is ArrayBuffer | ArrayBufferView {
  return value instanceof ArrayBuffer || ArrayBuffer.isView(value);
}

function isBase64Url(value: unknown): value is string {
  return typeof value === 'string' && BASE64URL.test(value) && value.length % 4 !== 1;
}

function memberOf(value: unknown, member: string): unknown {
  return typeof value === 'object' && value !== null ? Reflect.get(value, member) : undefined;
}
