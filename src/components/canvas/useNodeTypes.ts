/**
 * The desktop builder's fetch of the node-type catalog: the one place the
 * canvas folder reaches the `node_types_list` Tauri command, which projects
 * the Rust `NodeTypeRegistry` (task P3.1).
 *
 * The catalog is static for a given build, so it is fetched once and shared
 * by every canvas via a module-level promise.
 */
import { useEffect, useState } from 'react';
import { listNodeTypes } from '../../lib/workflows';
import type { NodeTypeInfo } from './nodeCatalog';

/** Shared in-flight/settled fetch — the catalog can't change within a build. */
let cached: Promise<NodeTypeInfo[]> | null = null;

export function loadNodeTypes(): Promise<NodeTypeInfo[]> {
  cached ??= listNodeTypes().catch((err) => {
    // Let a later mount retry rather than caching the failure forever.
    cached = null;
    throw err;
  });
  return cached;
}

/** Test seam: drop the memoized catalog between cases. */
export function resetNodeTypeCache(): void {
  cached = null;
}

export interface NodeTypesState {
  nodeTypes: NodeTypeInfo[];
  loading: boolean;
  error: string | null;
}

/** Fetch the catalog once per app run; every canvas shares the result. */
export function useNodeTypes(): NodeTypesState {
  const [state, setState] = useState<NodeTypesState>({
    nodeTypes: [],
    loading: true,
    error: null,
  });

  useEffect(() => {
    let alive = true;
    loadNodeTypes()
      .then((nodeTypes) => {
        if (alive) setState({ nodeTypes, loading: false, error: null });
      })
      .catch((err) => {
        if (alive) {
          setState({ nodeTypes: [], loading: false, error: String(err) });
        }
      });
    return () => {
      alive = false;
    };
  }, []);

  return state;
}
