/**
 * Pure graph mutations for design mode (task P3.1): every edit the canvas
 * makes to a schema-v2 definition, expressed as `def → def'` so the whole
 * builder can be unit tested without React Flow, and so P3.3's undo/redo has
 * plain immutable snapshots to push.
 *
 * Nothing here validates — `connectRules.ts` owns that, and the canvas asks
 * it *before* calling these. Keeping the two apart means a rule change never
 * has to touch the mutation code (and the rejection message can be rendered
 * without half-applying an edit).
 */
import type { NodeTypeInfo } from './nodeCatalog';
import type { NodeConfigV2, PositionV2, WorkflowDefinitionV2 } from './types';

/**
 * A readable, collision-free id for a new node of `kind`: `agent`,
 * `agent-2`, `agent-3`… Ids are the run-table key (`step_executions.step_id`)
 * and appear in retry redirect targets, so they need to stay legible rather
 * than becoming uuids.
 */
export function nextNodeId(def: WorkflowDefinitionV2, kind: string): string {
  const taken = new Set(def.nodes.map((n) => n.id));
  if (!taken.has(kind)) return kind;
  for (let i = 2; ; i += 1) {
    const candidate = `${kind}-${i}`;
    if (!taken.has(candidate)) return candidate;
  }
}

/** A node's default title: the type label, disambiguated by its id suffix. */
function defaultTitle(type: NodeTypeInfo, id: string): string {
  const suffix = id.slice(type.kind.length); // '' or '-2'
  return suffix ? `${type.label}${suffix.replace('-', ' ')}` : type.label;
}

export interface AddNodeResult {
  def: WorkflowDefinitionV2;
  /** Id of the node just added — the canvas selects it so the config panel
   *  (P3.2) opens on it immediately. */
  nodeId: string;
}

/**
 * Drop a new node of `type` at `position`, optionally wiring an edge from
 * `connectFrom` (the drag-from-handle-into-empty-canvas flow, PRD §6.3).
 *
 * The caller has already checked `atInstanceCap` / `canConnect`.
 */
export function addNode(
  def: WorkflowDefinitionV2,
  type: NodeTypeInfo,
  position: PositionV2,
  connectFrom?: string | null,
): AddNodeResult {
  const id = nextNodeId(def, type.kind);
  const node: NodeConfigV2 = {
    id,
    type: type.kind,
    title: defaultTitle(type, id),
    config: {},
    position,
  };
  return {
    def: {
      ...def,
      nodes: [...def.nodes, node],
      edges: connectFrom ? [...def.edges, { from: connectFrom, to: id }] : def.edges,
    },
    nodeId: id,
  };
}

/** Add an edge. Caller has already run `canConnect`. */
export function connectNodes(
  def: WorkflowDefinitionV2,
  from: string,
  to: string,
): WorkflowDefinitionV2 {
  return { ...def, edges: [...def.edges, { from, to }] };
}

/**
 * Remove a node and everything that referenced it: its edges, and any retry
 * redirect aimed at it. Leaving a dangling `redirect_to` behind is exactly
 * the audit-F39 bug class the builder is meant to make impossible, so the
 * cleanup happens here rather than being left for lint to complain about.
 */
export function removeNode(def: WorkflowDefinitionV2, nodeId: string): WorkflowDefinitionV2 {
  return {
    ...def,
    nodes: def.nodes.filter((n) => n.id !== nodeId).map((n) => stripRedirectsTo(n, nodeId)),
    edges: def.edges.filter((e) => e.from !== nodeId && e.to !== nodeId),
  };
}

function stripRedirectsTo(node: NodeConfigV2, target: string): NodeConfigV2 {
  if (!node.retry) return node;
  const classes = ['environment', 'verdict', 'agent_failure', 'non_retryable'] as const;
  let changed = false;
  const retry = { ...node.retry };
  for (const cls of classes) {
    const rule = retry[cls];
    if (rule?.redirect_to === target) {
      // The target is gone, so the redirect can't stand. Falling back to
      // `fail` is the conservative reading: it stops the run rather than
      // silently retrying in place somewhere the author never chose.
      retry[cls] = { ...rule, strategy: 'fail', redirect_to: null };
      changed = true;
    }
  }
  return changed ? { ...node, retry } : node;
}

/** Remove one edge. */
export function removeEdge(
  def: WorkflowDefinitionV2,
  from: string,
  to: string,
): WorkflowDefinitionV2 {
  return { ...def, edges: def.edges.filter((e) => !(e.from === from && e.to === to)) };
}

/**
 * Persist dragged positions back into the definition (PRD §5.1 co-persists
 * layout with the graph). Takes the whole map so a multi-select drag or an
 * elk auto-layout lands as one edit.
 */
export function moveNodes(
  def: WorkflowDefinitionV2,
  positions: Record<string, PositionV2>,
): WorkflowDefinitionV2 {
  return {
    ...def,
    nodes: def.nodes.map((n) => (positions[n.id] ? { ...n, position: positions[n.id] } : n)),
  };
}
