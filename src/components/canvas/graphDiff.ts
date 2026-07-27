/**
 * Structural diff between two schema-v2 graphs (task P3.4, PRD §6.3).
 *
 * The version drawer's job is to answer "what changed between v3 and what I
 * have now" on the canvas itself, which needs two things this module provides
 * and nothing else does:
 *
 *  - a per-node/per-edge verdict (`added | removed | changed | unchanged`) the
 *    canvas can tint from, and
 *  - a **union graph** to render it on, because a node that v3 had and the
 *    working copy doesn't exists in neither definition alone — without the
 *    merge, "removed" would be the one change the diff couldn't show.
 *
 * Two deliberate calls:
 *
 *  - **Position is not structure.** Moving a node changes the stored
 *    definition, but calling that "changed" would light up the whole canvas
 *    after an auto-layout and bury the edit that actually mattered. A
 *    position-only difference is reported as `moved` on an otherwise
 *    `unchanged` node.
 *  - **Absent, `null`, and `undefined` are the same value.** The two sides come
 *    from different producers — one from the Rust migration, one from the
 *    editor's own `graphEdits` — and they disagree about whether an empty
 *    optional is omitted or serialized as `null`. Treating that as a change
 *    would make every stored version look different from the graph it
 *    round-tripped into.
 */
import { edgeKey } from './lint';
import type { EdgeConfigV2, NodeConfigV2, WorkflowDefinitionV2 } from './types';

export type NodeDiffStatus = 'added' | 'removed' | 'changed' | 'unchanged';
export type EdgeDiffStatus = 'added' | 'removed' | 'changed' | 'unchanged';

/** A verdict worth drawing. `unchanged` is deliberately not one: a compare
 *  view marks what moved, and tinting everything else would bury it. */
export type NodeDiffMark = Exclude<NodeDiffStatus, 'unchanged'>;

export interface NodeDiff {
  status: NodeDiffStatus;
  /** Which structural fields differ — the drawer's per-node detail line. */
  fields: string[];
  /** Layout-only difference; never on its own a `changed` verdict. */
  moved: boolean;
}

export interface GraphDiff {
  nodes: Map<string, NodeDiff>;
  /** Keyed by `edgeKey(from, to)` — the same id `flowGraph` mints. */
  edges: Map<string, EdgeDiffStatus>;
  added: string[];
  removed: string[];
  changed: string[];
  /** Nothing structural differs (layout moves don't count). */
  identical: boolean;
}

/** The node fields a diff compares, in the order the detail line reads. */
const COMPARED_FIELDS = ['type', 'type_version', 'title', 'config', 'retry', 'join'] as const;

/**
 * Value in a form where absent / `null` / `undefined` collapse together and
 * object key order stops mattering, so `JSON.stringify` is a sound equality.
 */
function canonical(value: unknown): unknown {
  if (value === undefined || value === null) return null;
  if (Array.isArray(value)) return value.map(canonical);
  if (typeof value === 'object') {
    const source = value as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(source).sort()) {
      const v = canonical(source[key]);
      if (v === null) continue; // an empty optional reads as absent
      out[key] = v;
    }
    return out;
  }
  return value;
}

function same(a: unknown, b: unknown): boolean {
  return JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));
}

function fieldsThatDiffer(from: NodeConfigV2, to: NodeConfigV2): string[] {
  return COMPARED_FIELDS.filter(
    (field) => !same(from[field as keyof NodeConfigV2], to[field as keyof NodeConfigV2]),
  );
}

function byId(nodes: NodeConfigV2[]): Map<string, NodeConfigV2> {
  return new Map(nodes.map((n) => [n.id, n]));
}

function edgesByKey(edges: EdgeConfigV2[]): Map<string, EdgeConfigV2> {
  return new Map(edges.map((e) => [edgeKey(e.from, e.to), e]));
}

/**
 * Diff `from` (the older version) against `to` (the newer version, or the
 * working copy). Verdicts are phrased from `to`'s point of view: a node only
 * `from` has was *removed*, one only `to` has was *added*.
 */
export function diffGraphs(
  from: WorkflowDefinitionV2,
  to: WorkflowDefinitionV2,
): GraphDiff {
  const fromNodes = byId(from.nodes);
  const toNodes = byId(to.nodes);
  const nodes = new Map<string, NodeDiff>();
  const added: string[] = [];
  const removed: string[] = [];
  const changed: string[] = [];

  for (const node of to.nodes) {
    const before = fromNodes.get(node.id);
    if (!before) {
      nodes.set(node.id, { status: 'added', fields: [], moved: false });
      added.push(node.id);
      continue;
    }
    const fields = fieldsThatDiffer(before, node);
    const moved = !same(before.position, node.position);
    if (fields.length > 0) {
      nodes.set(node.id, { status: 'changed', fields, moved });
      changed.push(node.id);
    } else {
      nodes.set(node.id, { status: 'unchanged', fields: [], moved });
    }
  }

  for (const node of from.nodes) {
    if (toNodes.has(node.id)) continue;
    nodes.set(node.id, { status: 'removed', fields: [], moved: false });
    removed.push(node.id);
  }

  const fromEdges = edgesByKey(from.edges);
  const toEdges = edgesByKey(to.edges);
  const edges = new Map<string, EdgeDiffStatus>();

  for (const [key, edge] of toEdges) {
    const before = fromEdges.get(key);
    if (!before) edges.set(key, 'added');
    // Same endpoints, different guard: the branch is still there but takes a
    // different condition, which is a change worth seeing.
    else edges.set(key, same(before.when, edge.when) ? 'unchanged' : 'changed');
  }
  for (const key of fromEdges.keys()) {
    if (!toEdges.has(key)) edges.set(key, 'removed');
  }

  const edgeChanged = [...edges.values()].some((s) => s !== 'unchanged');

  return {
    nodes,
    edges,
    added,
    removed,
    changed,
    identical: added.length === 0 && removed.length === 0 && changed.length === 0 && !edgeChanged,
  };
}

/**
 * The graph to render a diff on: everything in `to`, plus what only `from`
 * had, so removed nodes and edges have somewhere to be drawn. Removed nodes
 * keep the position the older version gave them.
 *
 * Read-only by construction — nothing edits this graph; the working copy the
 * builder edits is untouched.
 */
export function mergeForDiff(
  from: WorkflowDefinitionV2,
  to: WorkflowDefinitionV2,
): WorkflowDefinitionV2 {
  const present = new Set(to.nodes.map((n) => n.id));
  const gone = from.nodes.filter((n) => !present.has(n.id));
  const keys = new Set(to.edges.map((e) => edgeKey(e.from, e.to)));
  const goneEdges = from.edges.filter((e) => !keys.has(edgeKey(e.from, e.to)));

  return {
    ...to,
    nodes: [...to.nodes, ...gone],
    edges: [...to.edges, ...goneEdges],
  };
}

/** `"2 added · 1 removed · 3 changed"`, or a plain statement of no change. */
export function diffSummary(diff: GraphDiff): string {
  const parts: string[] = [];
  if (diff.added.length > 0) parts.push(`${diff.added.length} added`);
  if (diff.removed.length > 0) parts.push(`${diff.removed.length} removed`);
  if (diff.changed.length > 0) parts.push(`${diff.changed.length} changed`);
  if (parts.length === 0) {
    const movedOnly = [...diff.nodes.values()].some((n) => n.moved);
    return movedOnly ? 'Layout only — no structural changes' : 'No structural changes';
  }
  return parts.join(' · ');
}
