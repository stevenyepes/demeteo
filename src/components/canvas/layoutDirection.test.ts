/**
 * The orientation heuristic is what stops a migrated pipeline from rendering
 * as a thin column down the middle of a wide window, so its verdicts are
 * pinned here rather than left to be eyeballed on a 4K display.
 */
import { describe, expect, it } from 'vitest';

import {
  graphBoxHeight,
  graphContainer,
  MAX_ZOOM as CANVAS_MAX_ZOOM,
  MINIMAP_MIN_SCALE,
  MINIMAP_NODE_THRESHOLD,
  MIN_GRAPH_BOX_PX,
  needsMiniMap,
  pickDirection,
  planLayout,
  type ContainerSize,
} from './layoutDirection';
import type { LayoutEdge, LayoutNode } from './useElkLayout';

const MAX_ZOOM = 1.75;

/** A linear chain of `n` nodes — the shape every v1→v2 migration produces. */
function chain(n: number): { nodes: LayoutNode[]; edges: LayoutEdge[] } {
  const nodes = Array.from({ length: n }, (_, i) => ({ id: `n${i}` }));
  const edges = Array.from({ length: n - 1 }, (_, i) => ({
    id: `e${i}`,
    source: `n${i}`,
    target: `n${i + 1}`,
  }));
  return { nodes, edges };
}

/** One source fanning out to `n` siblings that rejoin — wide, not deep. */
function fan(n: number): { nodes: LayoutNode[]; edges: LayoutEdge[] } {
  const nodes: LayoutNode[] = [{ id: 'start' }, { id: 'end' }];
  const edges: LayoutEdge[] = [];
  for (let i = 0; i < n; i++) {
    nodes.push({ id: `b${i}` });
    edges.push({ id: `in${i}`, source: 'start', target: `b${i}` });
    edges.push({ id: `out${i}`, source: `b${i}`, target: 'end' });
  }
  return { nodes, edges };
}

const WIDE: ContainerSize = { width: 2400, height: 700 };
const LAPTOP: ContainerSize = { width: 1100, height: 620 };
const TALL: ContainerSize = { width: 700, height: 1400 };
/** A full-screen 4K window — the whole scrolling run column, chrome included.
 *  This is the *measurement*, not a container any graph box ever gets: the
 *  4K verdicts below all start here and go through `graphContainer` first. */
const FOUR_K: ContainerSize = { width: 3400, height: 1600 };
/** Measured height of the run-event timeline, stepper, gate table and
 *  Graph|Timeline toggle stacked above the graph in that same column. */
const FOUR_K_CHROME = 700;
/** The graph box inside that window — 4K width, box-shaped height. Derived by
 *  the same pure helper `FeatureDetail` calls, so the fixture cannot state a
 *  premise the caller doesn't. */
const FOUR_K_BOX = graphContainer(FOUR_K, FOUR_K_CHROME)!;
/** A container that genuinely has a full column's height available — same
 *  numbers as `FOUR_K`, deliberately a separate name. It is *not* the 4K
 *  window verdict (that one is `RIGHT`); it is the counterfactual the
 *  subtraction exists to avoid, kept so the contrast stays pinned. */
const FULL_HEIGHT_CONTAINER: ContainerSize = { width: FOUR_K.width, height: FOUR_K.height };

describe('pickDirection', () => {
  it('turns a migrated chain sideways in a wide window', () => {
    const { nodes, edges } = chain(7);
    expect(pickDirection(nodes, edges, WIDE, MAX_ZOOM)).toBe('RIGHT');
  });

  it('leaves that same chain vertical on a laptop, where sideways would shrink it', () => {
    // 7 cards side by side need ~2060px; in an 1100px box that's a 0.48 zoom
    // against 0.67 for the column. Filling the width isn't worth the cards
    // getting smaller — which is why the rule is "renders largest", not
    // "matches the container's aspect".
    const { nodes, edges } = chain(7);
    expect(pickDirection(nodes, edges, LAPTOP, MAX_ZOOM)).toBe('DOWN');
  });

  it('keeps a chain vertical in a portrait container', () => {
    const { nodes, edges } = chain(7);
    expect(pickDirection(nodes, edges, TALL, MAX_ZOOM)).toBe('DOWN');
  });

  it('keeps a wide fan-out vertical — turning it would make it taller than the box', () => {
    const { nodes, edges } = fan(8);
    expect(pickDirection(nodes, edges, WIDE, MAX_ZOOM)).toBe('DOWN');
  });

  it('leaves a graph small enough to fit either way alone', () => {
    // Both orientations clear `maxZoom`, so neither renders larger and the
    // persisted vertical shape wins by default.
    const { nodes, edges } = chain(2);
    expect(pickDirection(nodes, edges, WIDE, MAX_ZOOM)).toBe('DOWN');
  });

  it('falls back to the persisted orientation before the canvas is measured', () => {
    const { nodes, edges } = chain(7);
    expect(pickDirection(nodes, edges, null, MAX_ZOOM)).toBe('DOWN');
    expect(pickDirection(nodes, edges, { width: 0, height: 0 }, MAX_ZOOM)).toBe('DOWN');
    expect(pickDirection([], [], WIDE, MAX_ZOOM)).toBe('DOWN');
  });

  it('uses measured card sizes when React Flow has them', () => {
    // Unusually tall cards make a vertical stack far worse than the default
    // estimate would, so the sideways verdict has to survive on real numbers.
    const { edges } = chain(5);
    const nodes = Array.from({ length: 5 }, (_, i) => ({
      id: `n${i}`,
      measured: { width: 240, height: 220 },
    }));
    expect(pickDirection(nodes, edges, WIDE, MAX_ZOOM)).toBe('RIGHT');
  });

  it('terminates on a cycle instead of recursing forever', () => {
    const nodes = [{ id: 'a' }, { id: 'b' }, { id: 'c' }];
    const edges = [
      { id: '1', source: 'a', target: 'b' },
      { id: '2', source: 'b', target: 'c' },
      { id: '3', source: 'c', target: 'a' },
    ];
    expect(() => pickDirection(nodes, edges, WIDE, MAX_ZOOM)).not.toThrow();
  });

  it('ignores edges pointing at nodes that aren’t on the canvas', () => {
    const nodes = [{ id: 'a' }, { id: 'b' }];
    const edges = [
      { id: '1', source: 'a', target: 'b' },
      { id: '2', source: 'b', target: 'ghost' },
    ];
    expect(() => pickDirection(nodes, edges, WIDE, MAX_ZOOM)).not.toThrow();
  });
});

describe('graphContainer', () => {
  it('hands the plan the space the box has, not the column’s', () => {
    expect(graphContainer(FOUR_K, FOUR_K_CHROME)).toEqual({ width: 3400, height: 900 });
  });

  it('turns a migrated chain sideways at 4K, starting from the window measurement', () => {
    // AC-3, pinned end-to-end the way `FeatureDetail` runs it: measure the run
    // column, subtract the chrome above the graph, plan against what's left,
    // size the box from that plan. Nothing here is a hand-picked box literal.
    const { nodes, edges } = chain(7);
    const box = graphContainer(FOUR_K, FOUR_K_CHROME)!;
    const plan = planLayout(nodes, edges, box, CANVAS_MAX_ZOOM);

    expect(plan.direction).toBe('RIGHT');
    const height = graphBoxHeight(plan, box.height);
    expect(height).toBeLessThan(FOUR_K.height);
    expect(height).toBeGreaterThanOrEqual(MIN_GRAPH_BOX_PX);
  });

  it('is what separates that verdict from a container with the whole column free', () => {
    // The counterfactual, not the 4K window: given all 1600px, the stack fits
    // at ~1.73 against ~1.48 for the row, so "renders largest" says DOWN and
    // the box eats every pixel. That is the shape the subtraction removes.
    const { nodes, edges } = chain(7);
    const plan = planLayout(nodes, edges, FULL_HEIGHT_CONTAINER, CANVAS_MAX_ZOOM);
    expect(plan.direction).toBe('DOWN');
    expect(graphBoxHeight(plan, FULL_HEIGHT_CONTAINER.height)).toBe(FULL_HEIGHT_CONTAINER.height);
  });

  it('never returns a box shorter than the CSS floor, however tall the chrome', () => {
    expect(graphContainer(FOUR_K, 99_999)).toEqual({ width: 3400, height: MIN_GRAPH_BOX_PX });
    expect(graphContainer(LAPTOP, LAPTOP.height)).toEqual({
      width: LAPTOP.width,
      height: MIN_GRAPH_BOX_PX,
    });
    // Chrome taller than the column is an ordinary mid-resize reading, so it
    // has to answer with a usable box rather than a negative one.
    for (const chrome of [FOUR_K.height, FOUR_K.height + 1, FOUR_K.height * 10]) {
      expect(graphContainer(FOUR_K, chrome)!.height).toBeGreaterThanOrEqual(MIN_GRAPH_BOX_PX);
    }
  });

  it('passes an unmeasured column straight through as “nothing to plan for”', () => {
    // Every degenerate input answers rather than throwing — chrome is measured
    // one tick after the column, so both nulls are ordinary startup states.
    expect(graphContainer(null, 0)).toBeNull();
    expect(graphContainer({ width: 0, height: 0 }, 0)).toBeNull();
    expect(graphContainer({ width: 3400, height: 0 }, 100)).toBeNull();
    expect(graphContainer(LAPTOP, -50)).toEqual(LAPTOP);
    expect(graphContainer(LAPTOP, Number.NaN)).toEqual(LAPTOP);
  });

  it('keeps the box inside the space it was given at laptop size', () => {
    // The box is `shrink-0` with a computed height, so a plan sized against
    // more room than it has is what pushed it past the fold.
    const { nodes, edges } = chain(7);
    const box = graphContainer(LAPTOP, 120)!;
    const plan = planLayout(nodes, edges, box, CANVAS_MAX_ZOOM);
    expect(graphBoxHeight(plan, box.height)).toBeLessThanOrEqual(box.height);
    expect(box.height).toBeLessThan(LAPTOP.height);
  });
});

describe('planLayout', () => {
  it('turns a migrated chain sideways in a 4K graph box', () => {
    const { nodes, edges } = chain(7);
    const plan = planLayout(nodes, edges, FOUR_K_BOX, CANVAS_MAX_ZOOM);
    expect(plan.direction).toBe('RIGHT');
    // The sideways box is one row of 7 cards: wide, and only a card tall.
    expect(plan.aspect).toBeGreaterThan(1);
    expect(plan.graph.width).toBeGreaterThan(plan.graph.height);
    expect(plan.fitScale).toBeGreaterThan(0);
  });

  it('agrees with pickDirection on every pinned verdict', () => {
    // pickDirection is now a wrapper, so this is the parity proof.
    const { nodes, edges } = chain(7);
    for (const container of [WIDE, LAPTOP, TALL, FOUR_K, FOUR_K_BOX]) {
      expect(planLayout(nodes, edges, container, MAX_ZOOM).direction).toBe(
        pickDirection(nodes, edges, container, MAX_ZOOM),
      );
    }
  });

  it('returns a zero plan rather than throwing when there is nothing to plan', () => {
    const { nodes, edges } = chain(7);
    for (const plan of [
      planLayout(nodes, edges, null, CANVAS_MAX_ZOOM),
      planLayout(nodes, edges, { width: 0, height: 0 }, CANVAS_MAX_ZOOM),
      planLayout([], [], FOUR_K, CANVAS_MAX_ZOOM),
    ]) {
      expect(plan.direction).toBe('DOWN');
      expect(plan.fitScale).toBe(0);
      expect(plan.aspect).toBe(0);
    }
  });
});

describe('graphBoxHeight', () => {
  it('gives a sideways chain only the height it needs, not the column’s leftover', () => {
    // The bug this closes: a ~64px row rendered in ~900px of empty canvas.
    const { nodes, edges } = chain(7);
    const plan = planLayout(nodes, edges, FOUR_K_BOX, CANVAS_MAX_ZOOM);
    const height = graphBoxHeight(plan, FOUR_K_BOX.height);
    expect(height).toBeLessThan(FOUR_K.height);
    expect(height).toBeGreaterThanOrEqual(MIN_GRAPH_BOX_PX);
  });

  it('never overflows the column it was given', () => {
    const { nodes, edges } = chain(7);
    const plan = planLayout(nodes, edges, LAPTOP, MAX_ZOOM);
    expect(graphBoxHeight(plan, LAPTOP.height)).toBeLessThanOrEqual(LAPTOP.height);
  });

  it('holds the 28rem floor when the column is shorter than it', () => {
    const { nodes, edges } = chain(7);
    const plan = planLayout(nodes, edges, FOUR_K_BOX, CANVAS_MAX_ZOOM);
    expect(graphBoxHeight(plan, MIN_GRAPH_BOX_PX - 200)).toBe(MIN_GRAPH_BOX_PX);
    expect(graphBoxHeight(plan, 0)).toBe(MIN_GRAPH_BOX_PX);
  });
});

describe('needsMiniMap', () => {
  it('stays out of the way of a small graph that renders large', () => {
    const { nodes, edges } = chain(3);
    expect(needsMiniMap(planLayout(nodes, edges, WIDE, MAX_ZOOM), nodes.length)).toBe(false);
  });

  it('appears for a graph that only fits at an illegible scale', () => {
    // Five nodes — under the count threshold — squeezed into a small box.
    const { nodes, edges } = chain(5);
    const plan = planLayout(nodes, edges, { width: 300, height: 200 }, CANVAS_MAX_ZOOM);
    expect(plan.fitScale).toBeLessThan(MINIMAP_MIN_SCALE);
    expect(nodes.length).toBeLessThan(MINIMAP_NODE_THRESHOLD);
    expect(needsMiniMap(plan, nodes.length)).toBe(true);
  });

  it('appears at the node count threshold even when the graph renders legibly', () => {
    const { nodes, edges } = chain(MINIMAP_NODE_THRESHOLD);
    const plan = planLayout(nodes, edges, WIDE, MAX_ZOOM);
    expect(plan.fitScale).toBeGreaterThanOrEqual(MINIMAP_MIN_SCALE);
    expect(needsMiniMap(plan, nodes.length)).toBe(true);
  });

  it('does not demand a minimap for an unmeasured canvas', () => {
    expect(needsMiniMap(planLayout([], [], null, CANVAS_MAX_ZOOM), 0)).toBe(false);
  });
});
