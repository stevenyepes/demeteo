/**
 * hub-web's whole network surface. It is served from the Hub's own origin and
 * talks to nothing else, so every request is a path — there is no host, port or
 * prefix to configure here. This file carries the verbs and the error shape of
 * the Hub's `/v1` API; the wrappers that name an endpoint sit beside it and
 * call through these.
 */

export type Guard<T> = (value: unknown) => value is T;

export class HubApiError extends Error {
  readonly path: string;
  /** HTTP status, or null when no response arrived. */
  readonly status: number | null;
  /** The `error` string of a non-2xx JSON body, or null when it carried none. */
  readonly code: string | null;
  /** The `scope` string of a non-2xx JSON body — the withheld scope of a `scope_not_granted` — or null. */
  readonly scope: string | null;

  constructor(
    message: string,
    path: string,
    status: number | null,
    code: string | null = null,
    scope: string | null = null,
  ) {
    super(message);
    this.name = 'HubApiError';
    this.path = path;
    this.status = status;
    this.code = code;
    this.scope = scope;
  }
}

export const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function rejectSyntheticInstance(instanceId: string, path: string): void {
  if (!UUID.test(instanceId)) {
    throw new HubApiError(`${instanceId} is not an instance id`, path, 404, 'not_found');
  }
}

/**
 * Held here and nowhere else — never `localStorage`, `sessionStorage` or a
 * cookie. The Hub derives the token from the session secret and hands it back
 * on `GET /v1/session`, so a reload re-reads it; a stored copy would outlive
 * the session it belongs to and be readable by anything that reads the store.
 */
let csrfToken: string | null = null;

export function setCsrfToken(token: string | null): void {
  csrfToken = token;
}

const sessionGoneListeners = new Set<() => void>();

/** Returns its unsubscribe. */
export function onSessionGone(listener: () => void): () => void {
  sessionGoneListeners.add(listener);
  return () => {
    sessionGoneListeners.delete(listener);
  };
}

/**
 * The one place a session that ended without a sign-out is acted on. The token
 * goes before anyone is told, so a listener that sends a request sends it as
 * signed out.
 */
export function dropSession(): void {
  csrfToken = null;
  for (const listener of [...sessionGoneListeners]) listener();
}

export function getJson<T>(path: string, guard: Guard<T>): Promise<T> {
  return request(path, undefined, guard);
}

export function postJson<T>(path: string, body: unknown, guard: Guard<T>): Promise<T> {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (csrfToken !== null) headers['X-CSRF-Token'] = csrfToken;
  return request(path, { method: 'POST', headers, body: JSON.stringify(body) }, guard);
}

/**
 * The `Blob` is the body as it stands, with no Content-Type set here: the Hub
 * reads the raw request body as the file and answers its SHA-256, so any
 * encoding — JSON, base64, a multipart envelope — would become part of the file.
 */
export function postBytes<T>(path: string, body: Blob, guard: Guard<T>): Promise<T> {
  const headers: Record<string, string> = {};
  if (csrfToken !== null) headers['X-CSRF-Token'] = csrfToken;
  return request(path, { method: 'POST', headers, body }, guard);
}

async function request<T>(path: string, init: RequestInit | undefined, guard: Guard<T>): Promise<T> {
  if (!isSameOriginPath(path)) {
    throw new HubApiError(`Not a same-origin path: ${path}`, path, null);
  }

  const heldWhenSent = csrfToken;
  let response: Response;
  try {
    response = await fetch(path, init);
  } catch (error) {
    throw new HubApiError(`Request to ${path} failed: ${reasonOf(error)}`, path, null);
  }
  if (response.status === 401 && endsSessionOf(heldWhenSent)) dropSession();
  if (!response.ok) {
    const { code, scope } = await errorOf(response);
    throw new HubApiError(`${path} answered HTTP ${response.status}`, path, response.status, code, scope);
  }

  let body: unknown;
  try {
    body = await response.json();
  } catch {
    throw new HubApiError(`${path} did not answer with JSON`, path, response.status);
  }
  if (!guard(body)) {
    throw new HubApiError(`${path} answered with an unexpected shape`, path, response.status);
  }
  return body;
}

/**
 * A `401` says the session is gone only to a request sent while one was held.
 * With no token it is a refused sign-in or the first `GET /v1/session`, and
 * both callers handle that themselves. With a token that is no longer the held
 * one, a sign-in finished (or another `401` was acted on) while this request
 * was in flight, and dropping now would undo it.
 */
function endsSessionOf(heldWhenSent: string | null): boolean {
  return heldWhenSent !== null && heldWhenSent === csrfToken;
}

async function errorOf(response: Response): Promise<{ code: string | null; scope: string | null }> {
  let body: unknown;
  try {
    body = await response.json();
  } catch {
    return { code: null, scope: null };
  }
  if (typeof body !== 'object' || body === null) return { code: null, scope: null };
  const code: unknown = Reflect.get(body, 'error');
  const scope: unknown = Reflect.get(body, 'scope');
  return {
    code: typeof code === 'string' ? code : null,
    scope: typeof scope === 'string' ? scope : null,
  };
}

/**
 * The backslash, tab and newline tests are not redundant with the `//` one:
 * the URL parser reads `\` as `/` and drops tabs and newlines before it
 * resolves, so `/\host` and `/<tab>/host` both leave the origin.
 */
function isSameOriginPath(path: string): boolean {
  return (
    path.startsWith('/') && !path.startsWith('//') && !path.includes('://') && !/[\\\t\n\r]/.test(path)
  );
}

function reasonOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
