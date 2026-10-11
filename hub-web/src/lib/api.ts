/**
 * hub-web's whole network surface. It is served from the Hub's own origin and
 * talks to nothing else, so every request is a path — there is no host, port or
 * prefix to configure here, and `crates/demeteo-hub` has no HTTP API yet for an
 * endpoint wrapper to name.
 */

export type Guard<T> = (value: unknown) => value is T;

export class HubApiError extends Error {
  readonly path: string;
  /** HTTP status, or null when no response arrived. */
  readonly status: number | null;

  constructor(message: string, path: string, status: number | null) {
    super(message);
    this.name = 'HubApiError';
    this.path = path;
    this.status = status;
  }
}

export async function getJson<T>(path: string, guard: Guard<T>): Promise<T> {
  if (!isSameOriginPath(path)) {
    throw new HubApiError(`Not a same-origin path: ${path}`, path, null);
  }

  let response: Response;
  try {
    response = await fetch(path);
  } catch (error) {
    throw new HubApiError(`Request to ${path} failed: ${reasonOf(error)}`, path, null);
  }
  if (!response.ok) {
    throw new HubApiError(`${path} answered HTTP ${response.status}`, path, response.status);
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
