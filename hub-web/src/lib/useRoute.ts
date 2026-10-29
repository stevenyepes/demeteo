import { useMemo, useSyncExternalStore } from 'react';

import { parseRoute, type Route } from './router';

function subscribe(onChange: () => void): () => void {
  window.addEventListener('hashchange', onChange);
  return () => window.removeEventListener('hashchange', onChange);
}

function readHash(): string {
  return window.location.hash;
}

/**
 * The snapshot is the hash string, not the parsed `Route`: `parseRoute` builds
 * a fresh object per call, and `useSyncExternalStore` compares snapshots with
 * `Object.is`, so a parsed snapshot reads as a store change on every render
 * and never settles.
 */
export function useRoute(): Route {
  const hash = useSyncExternalStore(subscribe, readHash);
  return useMemo(() => parseRoute(hash), [hash]);
}
