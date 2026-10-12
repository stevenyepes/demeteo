/**
 * The route lives in the URL fragment, not the path. `crates/demeteo-hub`
 * serves this bundle as plain files and promises no `index.html` fallback for
 * a deep link; a fragment never reaches the server, which keeps "serve this
 * directory" the whole of what routing asks of it. History routing becomes an
 * option only once the Hub commits to that fallback.
 */

export type Route =
  | { name: 'fleet' }
  | { name: 'instance'; instanceId: string }
  | { name: 'runs' }
  | { name: 'run'; instanceId: string; featureId: string }
  | { name: 'dispatch' }
  | { name: 'add-instance' }
  | { name: 'sign-up' }
  | { name: 'sign-in' }
  | { name: 'passkeys' }
  | { name: 'not-found'; path: string };

export function parseRoute(hash: string): Route {
  const path = hash.startsWith('#') ? hash.slice(1) : hash;
  return matchSegments(splitPath(path)) ?? { name: 'not-found', path };
}

export function routeHref(route: Exclude<Route, { name: 'not-found' }>): string {
  return `#/${segmentsOf(route).map((segment) => encodeURIComponent(segment)).join('/')}`;
}

function segmentsOf(route: Exclude<Route, { name: 'not-found' }>): string[] {
  switch (route.name) {
    case 'fleet':
    case 'runs':
    case 'dispatch':
    case 'add-instance':
    case 'sign-up':
    case 'sign-in':
    case 'passkeys':
      return [route.name];
    case 'instance':
      return ['instances', route.instanceId];
    case 'run':
      return ['instances', route.instanceId, 'runs', route.featureId];
  }
}

function splitPath(path: string): string[] | null {
  const segments = path.split('/');
  if (segments.shift() !== '') return null;
  if (segments[segments.length - 1] === '') segments.pop();
  try {
    return segments.map((segment) => decodeURIComponent(segment));
  } catch {
    return null;
  }
}

function matchSegments(segments: string[] | null): Route | null {
  if (segments === null || segments.includes('')) return null;
  const [head, instanceId, tail, featureId] = segments;
  if (segments.length === 0) return { name: 'fleet' };
  if (segments.length === 1) {
    switch (head) {
      case 'fleet':
      case 'runs':
      case 'dispatch':
      case 'add-instance':
      case 'sign-up':
      case 'sign-in':
      case 'passkeys':
        return { name: head };
      default:
        return null;
    }
  }
  if (head !== 'instances') return null;
  if (segments.length === 2) return { name: 'instance', instanceId };
  if (segments.length === 4 && tail === 'runs') return { name: 'run', instanceId, featureId };
  return null;
}
