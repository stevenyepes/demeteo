/**
 * The builder's node-type catalog: the wire shape of one entry of the Rust
 * `NodeTypeRegistry` (task P3.1), and the lookup the canvas rules do over it.
 *
 * PRD §6.3 requires the palette to *derive* from the registry so a new node
 * type — P3.5's `command`, later `subworkflow` — appears with zero frontend
 * edits. Nothing here enumerates kinds: the only per-kind knowledge the
 * frontend keeps is the lucide icon in `types.ts`, which already falls back
 * gracefully for a type it hasn't been taught about.
 *
 * This module imports nothing from `src/lib/`, and must not re-export from
 * `./useNodeTypes`. The Hub's browser UI (hub-web) reaches it through
 * `WorkflowCanvas`, and a browser has no Tauri IPC: any value import that
 * leads to `@tauri-apps/api` — a re-export counts — puts the desktop bridge
 * in the Hub's bundle. Whatever fetches the catalog belongs in
 * `./useNodeTypes`; code in this module is handed the entries.
 */

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

/** Index the catalog by kind for the per-node lookups the rules do. */
export function byKind(nodeTypes: NodeTypeInfo[]): Map<string, NodeTypeInfo> {
  return new Map(nodeTypes.map((t) => [t.kind, t]));
}
