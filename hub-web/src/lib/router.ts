/**
 * The route lives in the URL fragment, not the path. `crates/demeteo-hub` is
 * not built, so nothing promises an `index.html` fallback for a deep link; a
 * fragment never reaches the server, which keeps "serve this directory" the
 * whole server contract. History routing becomes an option only once the Hub
 * commits to that fallback.
 */

export type Route =
  | { name: 'fleet' }
  | { name: 'instance'; instanceId: string }
  | { name: 'runs' }
  | { name: 'run'; instanceId: string; featureId: string }
  | { name: 'dispatch' }
  | { name: 'add-instance' }
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
    if (head === 'fleet' || head === 'runs' || head === 'dispatch' || head === 'add-instance') {
      return { name: head };
    }
    return null;
  }
  if (head !== 'instances') return null;
  if (segments.length === 2) return { name: 'instance', instanceId };
  if (segments.length === 4 && tail === 'runs') return { name: 'run', instanceId, featureId };
  return null;
}
