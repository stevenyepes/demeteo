/**
 * `graphDiff` (task P3.4) — the pure half of the version drawer.
 *
 * The verdicts are what the canvas tints from, so the cases that matter are the
 * ones where a naive object comparison would lie: a node that only moved, a
 * field that is `null` on one side and absent on the other, and a removed node
 * that exists in neither definition on its own.
 */
import { describe, expect, it } from 'vitest';

import { diffGraphs, diffSummary, mergeForDiff } from './graphDiff';
import type { NodeConfigV2, WorkflowDefinitionV2 } from './types';

function node(id: string, extra: Partial<NodeConfigV2> = {}): NodeConfigV2 {
  return {
    id,
    type: 'agent',
    title: `Step ${id}`,
    config: { prompt_template: `do ${id}` },
    position: { x: 0, y: 0 },
    ...extra,
  };
}

function def(nodes: NodeConfigV2[], edges: { from: string; to: string; when?: string }[] = []): WorkflowDefinitionV2 {
  return { schema_version: 2, id: 'wf', name: 'Test', nodes, edges };
}

describe('diffGraphs', () => {
  it('reports a node only the newer side has as added', () => {
    const diff = diffGraphs(def([node('a')]), def([node('a'), node('b')]));
    expect(diff.added).toEqual(['b']);
    expect(diff.removed).toEqual([]);
    expect(diff.nodes.get('b')?.status).toBe('added');
    expect(diff.nodes.get('a')?.status).toBe('unchanged');
    expect(diff.identical).toBe(false);
  });

  it('reports a node only the older side has as removed', () => {
    const diff = diffGraphs(def([node('a'), node('b')]), def([node('a')]));
    expect(diff.removed).toEqual(['b']);
    expect(diff.nodes.get('b')?.status).toBe('removed');
  });

  it('names the fields that changed', () => {
    const diff = diffGraphs(
      def([node('a')]),
      def([node('a', { title: 'Renamed', config: { prompt_template: 'do it differently' } })]),
    );
    expect(diff.changed).toEqual(['a']);
    expect(diff.nodes.get('a')?.fields).toEqual(['title', 'config']);
  });

  it('treats a retry-policy edit as a change', () => {
    const diff = diffGraphs(
      def([node('a')]),
      def([node('a', { retry: { verdict: { strategy: 'redirect', redirect_to: 'a' } } })]),
    );
    expect(diff.nodes.get('a')?.fields).toEqual(['retry']);
  });

  it('calls a position-only difference moved, not changed', () => {
    const diff = diffGraphs(def([node('a')]), def([node('a', { position: { x: 400, y: 900 } })]));
    const a = diff.nodes.get('a');
    expect(a?.status).toBe('unchanged');
    expect(a?.moved).toBe(true);
    expect(diff.identical).toBe(true);
  });

  // The two sides come from different producers (the Rust migration vs the
  // editor's own edits) and disagree about how an empty optional is written.
  it('treats absent, null, and undefined as the same value', () => {
    const diff = diffGraphs(
      def([node('a', { retry: null, join: null })]),
      def([node('a', { config: { prompt_template: 'do a', model: null } })]),
    );
    expect(diff.nodes.get('a')?.status).toBe('unchanged');
    expect(diff.identical).toBe(true);
  });

  it('diffs edges by endpoint, and their guards by value', () => {
    const diff = diffGraphs(
      def([node('a'), node('b'), node('c')], [
        { from: 'a', to: 'b' },
        { from: 'b', to: 'c', when: '${{ nodes.b.outputs.verdict != \'FAIL\' }}' },
      ]),
      def([node('a'), node('b'), node('c')], [
        { from: 'a', to: 'c' },
        { from: 'b', to: 'c', when: '${{ nodes.b.outputs.verdict == \'PASS\' }}' },
      ]),
    );
    expect(diff.edges.get('a->b')).toBe('removed');
    expect(diff.edges.get('a->c')).toBe('added');
    expect(diff.edges.get('b->c')).toBe('changed');
    expect(diff.identical).toBe(false);
  });

  it('calls two copies of the same graph identical', () => {
    const graph = def([node('a'), node('b')], [{ from: 'a', to: 'b' }]);
    const diff = diffGraphs(graph, JSON.parse(JSON.stringify(graph)));
    expect(diff.identical).toBe(true);
    expect(diffSummary(diff)).toBe('No structural changes');
  });
});

describe('mergeForDiff', () => {
  it('keeps removed nodes and edges so the canvas can draw them', () => {
    const from = def([node('a'), node('gone')], [
      { from: 'a', to: 'gone' },
    ]);
    const to = def([node('a'), node('fresh')], [{ from: 'a', to: 'fresh' }]);

    const merged = mergeForDiff(from, to);
    expect(merged.nodes.map((n) => n.id)).toEqual(['a', 'fresh', 'gone']);
    expect(merged.edges.map((e) => `${e.from}->${e.to}`)).toEqual(['a->fresh', 'a->gone']);

    // And every merged element has a verdict to render with.
    const diff = diffGraphs(from, to);
    for (const n of merged.nodes) expect(diff.nodes.has(n.id)).toBe(true);
  });

  it('gives a removed node the position the older version had for it', () => {
    const from = def([node('gone', { position: { x: 42, y: 84 } })]);
    const merged = mergeForDiff(from, def([]));
    expect(merged.nodes[0].position).toEqual({ x: 42, y: 84 });
  });

  it('leaves the newer definition untouched', () => {
    const to = def([node('a')]);
    const before = JSON.stringify(to);
    mergeForDiff(def([node('b')]), to);
    expect(JSON.stringify(to)).toBe(before);
  });
});

describe('diffSummary', () => {
  it('counts each verdict', () => {
    const diff = diffGraphs(
      def([node('keep'), node('gone'), node('edit')]),
      def([node('keep'), node('edit', { title: 'Edited' }), node('new')]),
    );
    expect(diffSummary(diff)).toBe('1 added · 1 removed · 1 changed');
  });

  it('distinguishes a layout-only difference from no difference at all', () => {
    const moved = diffGraphs(def([node('a')]), def([node('a', { position: { x: 9, y: 9 } })]));
    expect(diffSummary(moved)).toBe('Layout only — no structural changes');
  });
});
