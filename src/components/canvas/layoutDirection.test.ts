/**
 * The orientation heuristic is what stops a migrated pipeline from rendering
 * as a thin column down the middle of a wide window, so its verdicts are
 * pinned here rather than left to be eyeballed on a 4K display.
 */
import { describe, expect, it } from 'vitest';

import { isUnarranged, pickDirection, type ContainerSize } from './layoutDirection';
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

describe('isUnarranged', () => {
  it('accepts the migration output — one column, every node at the same x', () => {
    expect(
      isUnarranged([
        { x: 0, y: 0 },
        { x: 0, y: 160 },
        { x: 0, y: 320 },
      ]),
    ).toBe(true);
  });

  it('accepts a graph nobody has positioned at all', () => {
    expect(isUnarranged([undefined, undefined])).toBe(true);
    expect(isUnarranged([])).toBe(true);
  });

  it('refuses a graph someone laid out by hand', () => {
    expect(
      isUnarranged([
        { x: 0, y: 0 },
        { x: 320, y: 160 },
        { x: 0, y: 320 },
      ]),
    ).toBe(false);
  });
});
