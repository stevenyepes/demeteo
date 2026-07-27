/**
 * Pure transform from a schema-v2 workflow definition to the `nodes`/`edges`
 * arrays React Flow renders. Kept side-effect-free and separate from the
 * `WorkflowCanvas` component so it can be exhaustively fixture-tested without
 * mounting React Flow (which needs a real layout engine jsdom lacks).
 *
 * Run-mode status overlay (task P2.2) rides on the optional `statusByNode`
 * map — design mode (P2.1) simply passes nothing.
 */
import type { Edge, Node } from '@xyflow/react';
import type { EdgeDiffStatus, GraphDiff, NodeDiffMark } from './graphDiff';
import { edgeKey, type LintIndex } from './lint';
import { isEssenceEmpty, nodeEssence, type NodeEssence } from './nodeSummary';
import type { NodeRunStatus, WorkflowDefinitionV2 } from './types';

/** Lint messages anchored to one node, split by severity (P3.3). */
export interface NodeLint {
  errors: string[];
  warnings: string[];
}

/** Data carried on each React Flow node into the `WorkflowNode` card. */
export interface WorkflowNodeData extends Record<string, unknown> {
  nodeType: string;
  title: string;
  /** Live run state (P2.2); undefined in design mode. */
  run?: NodeRunStatus;
  /** In the replay cone about to re-run (P2.4) — draws a "will re-run" ring. */
  highlighted?: boolean;
  /** Config-essence badges for design mode (P3.2); undefined in run mode,
   *  where the card's second row belongs to cost/duration instead. */
  essence?: NodeEssence;
  /** Structural-lint badge (P3.3); undefined when the node is clean. */
  lint?: NodeLint;
  /** Version-diff verdict (P3.4); undefined outside compare mode. */
  diff?: NodeDiffMark;
  /** Fields that differ, when `diff` is `changed` — the card's tooltip. */
  diffFields?: string[];
  /** Which edge of the card the handles sit on. Follows the elk direction so
   *  a left-to-right layout doesn't route its edges bottom-to-top. */
  orientation?: GraphOrientation;
}

/** `vertical` = handles top/bottom (elk `DOWN`); `horizontal` = left/right
 *  (elk `RIGHT`). */
export type GraphOrientation = 'vertical' | 'horizontal';

export type WorkflowFlowNode = Node<WorkflowNodeData, 'workflow'>;

/** Fallback layout stride when a node carries no persisted position — matches
 *  the migration's `VERTICAL_SPACING` so an unpositioned graph still reads as
 *  the same column before elk auto-layout runs. */
const FALLBACK_STRIDE = 160;

export interface ToFlowGraphOptions {
  /** node id → live run state, for run-mode overlay (P2.2). */
  statusByNode?: Record<string, NodeRunStatus>;
  /** node ids in the replay cone to highlight before confirming (P2.4). */
  highlightedNodeIds?: Set<string> | null;
  /** Design mode (P3.2): put each node's config essence on its card, so the
   *  graph is scannable without opening the config panel (PRD §6.3). */
  showEssence?: boolean;
  /** Structural-lint findings (P3.3) to badge nodes and tint edges with. */
  lint?: LintIndex;
  /** Version comparison (P3.4). Expects the *merged* graph from
   *  `mergeForDiff` as the definition, so removed nodes have a card to be
   *  drawn on. */
  diff?: GraphDiff;
  /** Handle placement; defaults to `vertical`, the migrated column's shape. */
  orientation?: GraphOrientation;
}

/** Edge stroke per worst anchored finding — a broken edge has to be findable
 *  on the canvas, not only in the save-blocked list. */
const EDGE_ERROR_STROKE = '#f43f5e'; // rose-500
const EDGE_WARNING_STROKE = '#f59e0b'; // amber-500

/** Diff stroke per edge verdict — same color language as the node cards. */
const EDGE_DIFF_STROKE: Partial<Record<EdgeDiffStatus, string>> = {
  added: '#10b981', // emerald-500
  removed: '#f43f5e', // rose-500
  changed: '#f59e0b', // amber-500
};

export function toFlowGraph(
  def: WorkflowDefinitionV2,
  opts: ToFlowGraphOptions = {},
): { nodes: WorkflowFlowNode[]; edges: Edge[] } {
  const nodes: WorkflowFlowNode[] = def.nodes.map((n, i) => {
    const essence = opts.showEssence ? nodeEssence(n) : null;
    const findings = opts.lint?.byNode.get(n.id) ?? [];
    const lint: NodeLint = {
      errors: findings.filter((f) => f.severity === 'error').map((f) => f.message),
      warnings: findings.filter((f) => f.severity === 'warning').map((f) => f.message),
    };
    const diff = opts.diff?.nodes.get(n.id);
    return {
      id: n.id,
      type: 'workflow' as const,
      position: n.position ?? { x: 0, y: i * FALLBACK_STRIDE },
      data: {
        nodeType: n.type,
        title: n.title,
        run: opts.statusByNode?.[n.id],
        highlighted: opts.highlightedNodeIds?.has(n.id) ?? false,
        essence: essence && !isEssenceEmpty(essence) ? essence : undefined,
        lint: findings.length > 0 ? lint : undefined,
        // `unchanged` carries no signal, so it stays off the card entirely —
        // a compare view should draw the eye to what moved.
        diff: diff && diff.status !== 'unchanged' ? diff.status : undefined,
        diffFields: diff?.fields.length ? diff.fields : undefined,
        orientation: opts.orientation ?? 'vertical',
      },
    };
  });

  const edges: Edge[] = def.edges.map((e) => {
    const key = edgeKey(e.from, e.to);
    const findings = opts.lint?.byEdge.get(key) ?? [];
    const worst = findings.some((f) => f.severity === 'error')
      ? 'error'
      : findings.length > 0
        ? 'warning'
        : null;
    // Lint wins the stroke where both apply: a broken edge is the one you must
    // act on. In practice compare mode passes no lint, so they don't collide.
    const diffStatus = opts.diff?.edges.get(key);
    const diffStroke = diffStatus ? EDGE_DIFF_STROKE[diffStatus] : undefined;
    return {
      id: key,
      source: e.from,
      target: e.to,
      // A conditional edge (a `when` guard) reads as a labeled branch.
      ...(e.when ? { label: 'when', data: { when: e.when } } : {}),
      ...(worst
        ? {
            style: {
              stroke: worst === 'error' ? EDGE_ERROR_STROKE : EDGE_WARNING_STROKE,
              strokeWidth: 2,
            },
            data: {
              ...(e.when ? { when: e.when } : {}),
              lint: findings.map((f) => f.message),
            },
          }
        : diffStroke
          ? {
              style: { stroke: diffStroke, strokeWidth: 2 },
              data: { ...(e.when ? { when: e.when } : {}), diff: diffStatus },
            }
          : {}),
    };
  });

  return { nodes, edges };
}
