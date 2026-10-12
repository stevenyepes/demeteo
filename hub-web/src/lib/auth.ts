import {
  dropSession,
  type Guard,
  getJson,
  HubApiError,
  onSessionGone,
  postJson,
  setCsrfToken,
} from './api';
import { createCredential, getCredential } from './webauthn';

export type Session = { csrfToken: string; credentialId: string };

type WireSession = { csrf_token: string; credential_id: string };

type CeremonyStarted = { ceremony_id: string; options: unknown };

/**
 * The passkey this tab signed in with, held as long as its token is and no
 * longer. The cookie is shared by every tab and this is not, which makes it
 * the one thing here that can tell a session read back from the cookie apart
 * from the session this page is showing.
 */
let signedInWith: string | null = null;

onSessionGone(() => {
  signedInWith = null;
});

/**
 * `null` means signed out, and only a `401` says that. A `500` or a request
 * that never got an answer says nothing about the session, so it is rethrown:
 * read as `null` it would put the sign-in view in front of a signed-in user.
 */
export async function fetchSession(): Promise<Session | null> {
  let wire: WireSession;
  try {
    wire = await getJson('/v1/session', isWireSession);
  } catch (error) {
    if (error instanceof HubApiError && error.status === 401) return null;
    throw error;
  }
  return established(wire);
}

export async function signUp(label: string): Promise<Session> {
  const started = await postJson('/v1/register/start', { label }, isCeremonyStarted);
  const credential = await createCredential(started.options);
  return established(
    await postJson('/v1/register/finish', { ceremony_id: started.ceremony_id, credential }, isWireSession),
  );
}

export async function signIn(): Promise<Session> {
  const started = await postJson('/v1/session/start', {}, isCeremonyStarted);
  const credential = await getCredential(started.options);
  return established(
    await postJson('/v1/session/finish', { ceremony_id: started.ceremony_id, credential }, isWireSession),
  );
}

/**
 * Resolves once no session is left to end, whichever side ended it: a `401`
 * means the Hub had already ended this one (expiry, or its passkey revoked).
 *
 * A CSRF `403` is not that. It says this tab's token is not the cookie's
 * session — another tab signed out and back in — and the cookie still names a
 * live session a reload would show. So the session is read again, which
 * installs the cookie's token, and the request is sent once more.
 *
 * Any other failure rejects with the token kept: a second attempt needs it.
 */
export async function signOut(): Promise<void> {
  try {
    await endSession();
  } catch (error) {
    if (!isCsrfRefusal(error)) throw error;
    if ((await fetchSession()) === null) return;
    await endSession();
  }
  setCsrfToken(null);
  signedInWith = null;
}

/**
 * `postJson` for a route that acts on the session's tenant. A CSRF `403` there
 * says what it says to `signOut`: this tab's token is not the cookie's session.
 * It is the route guard's answer — the handler never ran, so no ceremony was
 * taken and nothing was written — which is what makes sending the request
 * again safe. It is sent again once, and only into the session this tab signed
 * in with; whatever that attempt answers is the outcome.
 */
export async function postInSession<T>(path: string, body: unknown, guard: Guard<T>): Promise<T> {
  try {
    return await postJson(path, body, guard);
  } catch (refusal) {
    if (!isCsrfRefusal(refusal) || !(await sessionStillHeld())) throw refusal;
  }
  return postJson(path, body, guard);
}

/**
 * Reads the session again and says whether it is still this tab's, taking its
 * token if so. A `401` is not, and needs nothing done here: `api.ts` drops a
 * held session on that answer. Nor is a session of another passkey — the
 * cookie belongs to a different sign-in now, possibly another tenant's, and a
 * request sent with its token would act there (an `add/start` would enrol a
 * passkey in it), so that one is dropped here instead of adopted.
 *
 * Rejects when the read says nothing either way, as `fetchSession` does.
 */
export async function sessionStillHeld(): Promise<boolean> {
  const before = signedInWith;
  const session = await fetchSession();
  if (session === null) return false;
  if (session.credentialId === before) return true;
  dropSession();
  return false;
}

async function endSession(): Promise<void> {
  try {
    await postJson('/v1/session/end', {}, isJsonObject);
  } catch (error) {
    if (error instanceof HubApiError && error.status === 401) return;
    throw error;
  }
}

function isCsrfRefusal(error: unknown): boolean {
  return (
    error instanceof HubApiError &&
    error.status === 403 &&
    (error.code === 'csrf_mismatch' || error.code === 'csrf_required')
  );
}

function established(wire: WireSession): Session {
  setCsrfToken(wire.csrf_token);
  signedInWith = wire.credential_id;
  return { csrfToken: wire.csrf_token, credentialId: wire.credential_id };
}

const isWireSession: Guard<WireSession> = (value): value is WireSession =>
  typeof memberOf(value, 'csrf_token') === 'string' && typeof memberOf(value, 'credential_id') === 'string';

/** `options` is not looked at here: `webauthn.ts` owns that shape and refuses one it cannot use. */
const isCeremonyStarted: Guard<CeremonyStarted> = (value): value is CeremonyStarted =>
  typeof memberOf(value, 'ceremony_id') === 'string';

const isJsonObject: Guard<object> = (value): value is object => typeof value === 'object' && value !== null;

function memberOf(value: unknown, member: string): unknown {
  return typeof value === 'object' && value !== null ? Reflect.get(value, member) : undefined;
}
