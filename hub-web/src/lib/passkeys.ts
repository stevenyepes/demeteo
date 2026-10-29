import { dropSession, type Guard, getJson, HubApiError } from './api';
import { postInSession, sessionStillHeld } from './auth';
import { createCredential, getAssertion } from './webauthn';

export type PasskeyStatus = 'pending' | 'trusted' | 'revoked';

export type Passkey = {
  credentialId: string;
  label: string;
  status: PasskeyStatus;
  /** Unix seconds. */
  createdAt: number;
  /** Whether it is the passkey this session signed in with. */
  current: boolean;
};

type WirePasskey = {
  credential_id: string;
  label: string;
  status: PasskeyStatus;
  created_at: number;
  current: boolean;
};

type CeremonyStarted = { ceremony_id: string; options: unknown };

const REVOKE = '/v1/passkeys/revoke';

const STATUSES: readonly unknown[] = ['pending', 'trusted', 'revoked'] satisfies PasskeyStatus[];

export async function listPasskeys(): Promise<Passkey[]> {
  const { passkeys } = await getJson('/v1/passkeys', isPasskeyList);
  return passkeys.map(passkeyOf);
}

/** The passkey comes back `pending`: it signs nobody in until a trusted one endorses it. */
export async function addPasskey(label: string): Promise<Passkey> {
  const started = await postInSession('/v1/passkeys/add/start', { label }, isCeremonyStarted);
  const credential = await createCredential(started.options);
  const { passkey } = await postInSession(
    '/v1/passkeys/add/finish',
    { ceremony_id: started.ceremony_id, credential },
    isChangedPasskey,
  );
  return passkeyOf(passkey);
}

export function endorsePasskey(credentialId: string): Promise<Passkey> {
  return assertOver('/v1/passkeys/endorse', credentialId);
}

/**
 * Revoking the passkey this session signed in with ends the session in the
 * same transaction, and the answer is the last one the Hub gives it. Acted on
 * here, the page does not stay up as signed in until some later request is
 * answered `401`.
 *
 * That answer can be lost after the Hub committed. So when the finish fails
 * without the Hub having said it changed nothing, the session is asked for
 * before the failure is rethrown: gone, it is dropped by that read. The
 * failure is rethrown as it was either way — the read decides only whether the
 * shell stays up, and one that fails itself has decided nothing.
 */
export async function revokePasskey(credentialId: string): Promise<Passkey> {
  let changed: Passkey;
  try {
    changed = await assertOver(REVOKE, credentialId);
  } catch (error) {
    if (mayHaveRevoked(error)) await sessionStillHeld().catch(() => undefined);
    throw error;
  }
  if (changed.current && changed.status === 'revoked') dropSession();
  return changed;
}

/**
 * Only `revoke/finish` writes, and a `4xx` from it is a refusal: nothing was
 * written. No response, a `5xx`, and a `2xx` whose body could not be read all
 * leave that open.
 */
function mayHaveRevoked(error: unknown): boolean {
  if (!(error instanceof HubApiError) || error.path !== `${REVOKE}/finish`) return false;
  return error.status === null || error.status < 400 || error.status >= 500;
}

/**
 * `assertion` is the object `getAssertion` returned, untouched. The Hub stores
 * those four fields verbatim in the record it re-verifies later, so a member
 * renamed or re-encoded here is a record that no longer verifies.
 */
async function assertOver(
  route: '/v1/passkeys/endorse' | typeof REVOKE,
  credentialId: string,
): Promise<Passkey> {
  const started = await postInSession(
    `${route}/start`,
    { credential_id: credentialId },
    isCeremonyStarted,
  );
  const assertion = await getAssertion(started.options);
  const { passkey } = await postInSession(
    `${route}/finish`,
    { ceremony_id: started.ceremony_id, assertion },
    isChangedPasskey,
  );
  return passkeyOf(passkey);
}

function passkeyOf(wire: WirePasskey): Passkey {
  return {
    credentialId: wire.credential_id,
    label: wire.label,
    status: wire.status,
    createdAt: wire.created_at,
    current: wire.current,
  };
}

/**
 * A status this build does not know is refused rather than shown: the page
 * decides which of endorse and revoke to offer from it, and a guess there
 * offers a trust change on a passkey whose state it cannot name.
 */
const isWirePasskey: Guard<WirePasskey> = (value): value is WirePasskey =>
  typeof memberOf(value, 'credential_id') === 'string' &&
  typeof memberOf(value, 'label') === 'string' &&
  STATUSES.includes(memberOf(value, 'status')) &&
  typeof memberOf(value, 'created_at') === 'number' &&
  typeof memberOf(value, 'current') === 'boolean';

const isPasskeyList: Guard<{ passkeys: WirePasskey[] }> = (
  value,
): value is { passkeys: WirePasskey[] } => {
  const passkeys = memberOf(value, 'passkeys');
  return Array.isArray(passkeys) && passkeys.every(isWirePasskey);
};

const isChangedPasskey: Guard<{ passkey: WirePasskey }> = (
  value,
): value is { passkey: WirePasskey } => isWirePasskey(memberOf(value, 'passkey'));

const isCeremonyStarted: Guard<CeremonyStarted> = (value): value is CeremonyStarted =>
  typeof memberOf(value, 'ceremony_id') === 'string';

function memberOf(value: unknown, member: string): unknown {
  return typeof value === 'object' && value !== null ? Reflect.get(value, member) : undefined;
}
