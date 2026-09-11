/**
 * Which way a workflow graph should be laid out for the space it's given.
 *
 * A migrated pipeline is a single top-to-bottom column (`workflow_migrate.rs`
 * places every node at `x: 0`), which is the worst possible shape for the
 * landscape windows this app actually runs in: fit-view is height-bound, so
 * the graph fills the canvas vertically and leaves most of the width empty.
 *
 * Rather than hard-code an orientation, we estimate the bounding box elk's
 * `layered` algorithm would produce in each direction and keep the one that
 * renders *larger* in the current container. That criterion gets both ends
 * right on its own: a short chain in a wide box goes `RIGHT` and uses the
 * width, while a genuinely wide fan-out in a narrow box stays `DOWN`.
 *
 * The estimate only has to be good enough to order the two candidates — the
 * real coordinates still come from elk.
 */
import type { LayoutEdge, LayoutNode } from './useElkLayout';

export type LayoutDirection = 'DOWN' | 'RIGHT';

/** Card dimensions when React Flow hasn't measured a node yet. */
const DEFAULT_NODE = { width: 240, height: 64 };

/**
 * The box one node occupies as far as layout is concerned — its measurement
 * where React Flow has one, `DEFAULT_NODE` per missing axis otherwise.
 *
 * The single definition of a node's size: `useElkLayout` feeds elk exactly
 * this, the orientation estimate averages it, and `layoutSizeKey` fingerprints
 * it. A second fallback anywhere would let the fingerprint disagree with what
 * elk actually laid out.
 */
export function layoutNodeSize(node: LayoutNode): { width: number; height: number } {
  return {
    width: node.measured?.width ?? DEFAULT_NODE.width,
    height: node.measured?.height ?? DEFAULT_NODE.height,
  };
}

/** Mirrors `elk.spacing.nodeNode` (within a layer). */
const GAP_WITHIN = 48;
/** Mirrors `elk.layered.spacing.nodeNodeBetweenLayers` (across layers). */
const GAP_BETWEEN = 64;

/** Fit-view leaves a 10% margin, so the usable box is smaller than the DOM one.
 *  Exported as the single source of truth: `WorkflowCanvas` passes
 *  `padding: 1 - FIT_PADDING` to every `fitView` call, so the estimate below
 *  cannot drift from the margin React Flow actually applies. */
export const FIT_PADDING = 0.9;

/** React Flow's lower zoom bound. Single source of truth — `WorkflowCanvas`
 *  imports it for its `minZoom` prop instead of declaring its own. */
export const MIN_ZOOM = 0.2;

/**
 * React Flow's upper zoom bound, and the clamp `fitScale` estimates against.
 *
 * Single source of truth: `WorkflowCanvas` imports it for `maxZoom` and for
 * every explicit `fitView({ maxZoom })`, so the estimate here and the clamp
 * React Flow applies cannot drift apart.
 */
export const MAX_ZOOM = 2.4;

/** Floor for the graph box, mirroring the `min-h-[28rem]` the run column
 *  used before the height became computed. Single source of truth for the
 *  clamp in `graphBoxHeight`. */
export const MIN_GRAPH_BOX_PX = 448;

/** Below this fit scale the node labels stop being readable, so the minimap
 *  earns its space regardless of how few nodes there are. */
export const MINIMAP_MIN_SCALE = 0.55;

/** Node count at which the minimap appears on size alone — the pre-existing
 *  `WorkflowCanvas` threshold, now decided here with the rest of the plan. */
export const MINIMAP_NODE_THRESHOLD = 8;

/** `RIGHT` has to beat `DOWN` by this much to win. Without the margin, a
 *  near-tie flips orientation on a few pixels of resize. */
const FLIP_MARGIN = 1.08;

export interface ContainerSize {
  width: number;
  height: number;
}

/**
 * Longest-path layering — the same ranking elk's `layered` algorithm starts
 * from. Returns how many nodes land in each layer.
 *
 * Structurally the graph is a DAG (the Rust lint and `connectRules` both
 * refuse cycles), but this runs on live canvas state, so it tolerates one:
 * a node already being ranked caps its own recursion instead of hanging.
 */
function layerSizes(nodes: LayoutNode[], edges: LayoutEdge[]): number[] {
  const ids = new Set(nodes.map((n) => n.id));
  const incoming = new Map<string, string[]>();
  for (const e of edges) {
    if (!ids.has(e.source) || !ids.has(e.target)) continue;
    const from = incoming.get(e.target);
    if (from) from.push(e.source);
    else incoming.set(e.target, [e.source]);
  }

  const rank = new Map<string, number>();
  const inFlight = new Set<string>();
  const rankOf = (id: string): number => {
    const known = rank.get(id);
    if (known !== undefined) return known;
    if (inFlight.has(id)) return 0; // cycle guard
    inFlight.add(id);
    let depth = 0;
    for (const parent of incoming.get(id) ?? []) {
      depth = Math.max(depth, rankOf(parent) + 1);
    }
    inFlight.delete(id);
    rank.set(id, depth);
    return depth;
  };

  const counts: number[] = [];
  for (const n of nodes) {
    const r = rankOf(n.id);
    counts[r] = (counts[r] ?? 0) + 1;
  }
  return counts.map((c) => c ?? 0);
}

/** Extent of `count` cards stacked along one axis, including the gaps. */
function extent(count: number, cardSize: number, gap: number): number {
  return count <= 0 ? 0 : count * cardSize + (count - 1) * gap;
}

/**
 * How large the graph would render — the scale fit-view would settle on,
 * clamped to `maxZoom` so two orientations that both fit comfortably don't
 * differ on a zoom neither would actually use.
 */
function fitScale(
  graph: { width: number; height: number },
  container: ContainerSize,
  maxZoom: number,
): number {
  if (graph.width <= 0 || graph.height <= 0) return 0;
  const scale = Math.min(
    (container.width * FIT_PADDING) / graph.width,
    (container.height * FIT_PADDING) / graph.height,
  );
  return Math.min(scale, maxZoom);
}

export interface LayoutPlan {
  direction: LayoutDirection;
  /** Estimated elk bounding box in the chosen direction, unscaled CSS px. */
  graph: { width: number; height: number };
  /** The scale fit-view will settle on for `graph` in `container`. */
  fitScale: number;
  /** `graph.width / graph.height`; `0` for an empty graph. */
  aspect: number;
}

/** What `planLayout` answers when there is nothing to plan for — no container
 *  measured yet, or an empty graph. `DOWN` is the persisted/migrated
 *  orientation, so it is also the safe default. */
const EMPTY_PLAN: LayoutPlan = {
  direction: 'DOWN',
  graph: { width: 0, height: 0 },
  fitScale: 0,
  aspect: 0,
};

/**
 * Plan the orientation, box and minimap inputs for one graph in one container.
 *
 * The orientation criterion is unchanged: estimate the bounding box elk would
 * produce each way and keep whichever renders *larger*. What's new is that the
 * winning box and the scale it settles at come back with the verdict, because
 * the caller needs them too — a `RIGHT` chain is ~2000×64, and a box sized to
 * the column's leftover height would leave most of it empty.
 *
 * Falls back to `EMPTY_PLAN` whenever there isn't enough information to choose
 * (no container measured yet, empty graph).
 */
export function planLayout(
  nodes: LayoutNode[],
  edges: LayoutEdge[],
  container: ContainerSize | null,
  maxZoom: number,
): LayoutPlan {
  if (!container || container.width <= 0 || container.height <= 0) return EMPTY_PLAN;
  if (nodes.length === 0) return EMPTY_PLAN;

  const layers = layerSizes(nodes, edges);
  const depth = layers.length;
  const breadth = Math.max(...layers, 1);

  // Average card size: elk packs layers by actual node extents, and the
  // average is the closest single number to that without replaying its
  // placement pass.
  const cardWidth = nodes.reduce((sum, n) => sum + layoutNodeSize(n).width, 0) / nodes.length;
  const cardHeight = nodes.reduce((sum, n) => sum + layoutNodeSize(n).height, 0) / nodes.length;

  const down = {
    width: extent(breadth, cardWidth, GAP_WITHIN),
    height: extent(depth, cardHeight, GAP_BETWEEN),
  };
  const right = {
    width: extent(depth, cardWidth, GAP_BETWEEN),
    height: extent(breadth, cardHeight, GAP_WITHIN),
  };

  const downScale = fitScale(down, container, maxZoom);
  const rightScale = fitScale(right, container, maxZoom);
  const goRight = rightScale > downScale * FLIP_MARGIN;

  const graph = goRight ? right : down;
  return {
    direction: goRight ? 'RIGHT' : 'DOWN',
    graph,
    fitScale: goRight ? rightScale : downScale,
    aspect: graph.height > 0 ? graph.width / graph.height : 0,
  };
}

/**
 * The container the graph box actually gets, given the run column it lives in
 * and the height of the chrome stacked above it inside the same track.
 *
 * Planning against the whole column is what left a migrated pipeline vertical
 * on a 4K display: at 1600px of column height a 7-node stack still fits at
 * ~1.73, so `DOWN` "renders largest" — but the box never has 1600px, because
 * the stepper, the gate table and the Graph|Timeline toggle are above it. Feed
 * the plan the space the box has and the same window answers `RIGHT`.
 *
 * Pure and total: an unmeasured or empty column yields `null` (the caller's
 * "nothing to plan for"), and chrome taller than the column floors the box at
 * `MIN_GRAPH_BOX_PX` rather than going negative — the box has a CSS
 * `min-h-[28rem]` either way, so the plan should be told the same thing.
 */
export function graphContainer(
  column: ContainerSize | null,
  chromeHeight: number,
): ContainerSize | null {
  if (!column || column.width <= 0 || column.height <= 0) return null;
  const chrome = Number.isFinite(chromeHeight) && chromeHeight > 0 ? chromeHeight : 0;
  return { width: column.width, height: Math.max(column.height - chrome, MIN_GRAPH_BOX_PX) };
}

/**
 * Pick the orientation that renders largest in `container`.
 *
 * Falls back to `DOWN` — the persisted/migrated orientation — whenever there
 * isn't enough information to choose (no container measured yet, empty graph).
 */
export function pickDirection(
  nodes: LayoutNode[],
  edges: LayoutEdge[],
  container: ContainerSize | null,
  maxZoom: number,
): LayoutDirection {
  return planLayout(nodes, edges, container, maxZoom).direction;
}

/**
 * Height the graph box should take: enough for the graph at the scale fit-view
 * will settle on, including fit-view's own margin, clamped into
 * `[MIN_GRAPH_BOX_PX, max(availableHeight, MIN_GRAPH_BOX_PX)]`.
 *
 * This is what stops a `RIGHT` chain — one ~64px-tall row — from sitting in the
 * column's whole leftover height with ~900px of empty canvas under it. The
 * ceiling never goes below the floor, so a short column still renders the box
 * at its minimum rather than collapsing.
 */
export function graphBoxHeight(plan: LayoutPlan, availableHeight: number): number {
  const scaled = plan.graph.height * plan.fitScale;
  const desired = scaled > 0 ? Math.ceil(scaled / FIT_PADDING) : MIN_GRAPH_BOX_PX;
  const ceiling = Math.max(availableHeight, MIN_GRAPH_BOX_PX);
  return Math.min(Math.max(desired, MIN_GRAPH_BOX_PX), ceiling);
}

/**
 * Whether the minimap earns its space.
 *
 * Node count alone was the old rule; it misses the case the responsive box
 * introduces — a small graph that still fits only at a scale where the labels
 * are illegible, which is exactly when an overview helps.
 */
export function needsMiniMap(plan: LayoutPlan, nodeCount: number): boolean {
  if (nodeCount >= MINIMAP_NODE_THRESHOLD) return true;
  return plan.fitScale > 0 && plan.fitScale < MINIMAP_MIN_SCALE;
}

/**
 * Order-independent fingerprint of every node's layout box — `layoutNodeSize`,
 * nothing else. Positions and run status are deliberately absent: a status
 * tick re-seeds every node without resizing one, and must never re-run elk.
 */
export function layoutSizeKey(nodes: LayoutNode[]): string {
  return nodes
    .map((n) => {
      const { width, height } = layoutNodeSize(n);
      return `${n.id}:${width}x${height}`;
    })
    .sort()
    .join(',');
}

/** Whether React Flow has measured every node on both axes — the only state a
 *  `layoutSizeKey` is worth acting on. */
export function everyNodeMeasured(nodes: LayoutNode[]): boolean {
  return nodes.every((n) => n.measured?.width !== undefined && n.measured?.height !== undefined);
}

/** What the last successful auto-layout handed elk. */
export interface AppliedLayout {
  direction: LayoutDirection;
  sizeKey: string;
}

/**
 * Whether the run canvas re-runs elk or only re-fits the viewport.
 *
 * Elk places cards at the size they had when it ran, spaced `GAP_WITHIN` /
 * `GAP_BETWEEN` apart. A card that then grows — the assignment chips arriving
 * with `agent_spawned`, the cost row on completion — grows in place, and 80px
 * of growth into a 64px gap is an overlap. So a changed size is as much a
 * reason to lay out again as a changed direction; more room alone is not.
 */
export function autoLayoutAction(
  applied: AppliedLayout | null,
  direction: LayoutDirection,
  sizeKey: string,
): 'layout' | 'fit' {
  if (!applied) return 'layout';
  if (applied.direction !== direction) return 'layout';
  return applied.sizeKey === sizeKey ? 'fit' : 'layout';
}
