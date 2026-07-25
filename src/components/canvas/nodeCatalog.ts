/**
 * The builder's node-type catalog: a thin client over the `node_types_list`
 * Tauri command, which projects the Rust `NodeTypeRegistry` (task P3.1).
 *
 * PRD §6.3 requires the palette to *derive* from the registry so a new node
 * type — P3.5's `command`, later `subworkflow` — appears with zero frontend
 * edits. Nothing here enumerates kinds: the only per-kind knowledge the
 * frontend keeps is the lucide icon in `types.ts`, which already falls back
 * gracefully for a type it hasn't been taught about.
 *
 * The catalog is static for a given build, so it is fetched once and shared
 * by every canvas via a module-level promise.
 */
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

/** Coarse port type (mirrors Rust `PortType`, serde snake_case). */
export type PortType = 'text' | 'file' | 'task_list' | 'verdict' | 'approval' | 'any';

/** One palette entry — the serialized Rust `NodeTypeInfo`. */
export interface NodeTypeInfo {
  kind: string;
  label: string;
  summary: string;
  /** JSON Schema for the node's `config` payload; the config panel (P3.2)
   *  renders from it. Opaque here. */
  config_schema: Record<string, unknown>;
  inputs: PortType[];
  /** Empty means sink — nothing may connect out of it (`finalize`). */
  outputs: PortType[];
  /** Cap on instances per workflow; null = unbounded. */
  max_instances?: number | null;
}

/** Shared in-flight/settled fetch — the catalog can't change within a build. */
let cached: Promise<NodeTypeInfo[]> | null = null;

export function loadNodeTypes(): Promise<NodeTypeInfo[]> {
  cached ??= invoke<NodeTypeInfo[]>('node_types_list').catch((err) => {
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

/** Index the catalog by kind for the per-node lookups the rules do. */
export function byKind(nodeTypes: NodeTypeInfo[]): Map<string, NodeTypeInfo> {
  return new Map(nodeTypes.map((t) => [t.kind, t]));
}
