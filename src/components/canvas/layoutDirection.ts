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

/** Card dimensions when React Flow hasn't measured a node yet — same
 *  fallback `useElkLayout` feeds elk, so the estimate matches the result. */
const DEFAULT_NODE = { width: 240, height: 64 };
/** Mirrors `elk.spacing.nodeNode` (within a layer). */
const GAP_WITHIN = 48;
/** Mirrors `elk.layered.spacing.nodeNodeBetweenLayers` (across layers). */
const GAP_BETWEEN = 64;

/** Fit-view leaves a 10% margin, so the usable box is smaller than the DOM one. */
const FIT_PADDING = 0.9;

/** `RIGHT` has to beat `DOWN` by this much to win. Without the margin, a
 *  near-tie flips orientation on a few pixels of resize. */
const FLIP_MARGIN = 1.08;

export interface ContainerSize {
  width: number;
  height: number;
}

interface PositionLike {
  x: number;
  y: number;
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
  if (!container || container.width <= 0 || container.height <= 0) return 'DOWN';
  if (nodes.length === 0) return 'DOWN';

  const layers = layerSizes(nodes, edges);
  const depth = layers.length;
  const breadth = Math.max(...layers, 1);

  // Average card size: elk packs layers by actual node extents, and the
  // average is the closest single number to that without replaying its
  // placement pass.
  const cardWidth =
    nodes.reduce((sum, n) => sum + (n.measured?.width ?? DEFAULT_NODE.width), 0) / nodes.length;
  const cardHeight =
    nodes.reduce((sum, n) => sum + (n.measured?.height ?? DEFAULT_NODE.height), 0) / nodes.length;

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
  return rightScale > downScale * FLIP_MARGIN ? 'RIGHT' : 'DOWN';
}

/**
 * True when nobody has arranged this graph — every node sits on the same
 * column, which is exactly what the v1→v2 migration produces.
 *
 * Run mode auto-orients only these: a graph someone positioned by hand in the
 * builder is shown the way they left it.
 */
export function isUnarranged(positions: (PositionLike | null | undefined)[]): boolean {
  const known = positions.filter((p): p is PositionLike => p != null);
  if (known.length <= 1) return true;
  const first = known[0].x;
  return known.every((p) => Math.abs(p.x - first) < 1);
}
