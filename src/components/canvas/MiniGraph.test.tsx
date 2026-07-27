/**
 * `MiniGraph` (task P3.6): the launcher's shape preview.
 *
 * The claim under test is the one the flat per-step override list cannot make:
 * that structure is visible — a fan-out reads as two nodes on one rank, and a
 * chain reads as one per rank.
 */
import { render, screen, cleanup } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { MiniGraph, ranksOf } from './MiniGraph';
import type { WorkflowDefinitionV2 } from './types';

const def = (
  nodes: Array<[string, string]>,
  edges: Array<[string, string]>,
): WorkflowDefinitionV2 => ({
  schema_version: 2,
  id: 'wf',
  name: 'W',
  nodes: nodes.map(([id, type]) => ({ id, type, title: `Node ${id}` })),
  edges: edges.map(([from, to]) => ({ from, to })),
});

afterEach(cleanup);

describe('ranksOf', () => {
  it('puts a chain one node per rank, in order', () => {
    const ranks = ranksOf(
      def(
        [
          ['a', 'agent'],
          ['b', 'agent'],
          ['c', 'finalize'],
        ],
        [
          ['a', 'b'],
          ['b', 'c'],
        ],
      ),
    );
    expect(ranks).toEqual([['a'], ['b'], ['c']]);
  });

  it('puts independent branches on the same rank', () => {
    const ranks = ranksOf(
      def(
        [
          ['plan', 'agent'],
          ['left', 'agent'],
          ['right', 'agent'],
          ['ship', 'finalize'],
        ],
        [
          ['plan', 'left'],
          ['plan', 'right'],
          ['left', 'ship'],
          ['right', 'ship'],
        ],
      ),
    );
    expect(ranks[0]).toEqual(['plan']);
    expect(ranks[1].sort()).toEqual(['left', 'right']);
    expect(ranks[2]).toEqual(['ship']);
  });

  it('ranks a join below its deepest dependency, not its shallowest', () => {
    // `ship` depends on `a` (depth 0) and `c` (depth 2); a longest-path rank
    // keeps it below both, which is what makes the picture read as a dependency
    // order rather than an arbitrary grouping.
    const ranks = ranksOf(
      def(
        [
          ['a', 'agent'],
          ['b', 'agent'],
          ['c', 'agent'],
          ['ship', 'finalize'],
        ],
        [
          ['a', 'b'],
          ['b', 'c'],
          ['a', 'ship'],
          ['c', 'ship'],
        ],
      ),
    );
    expect(ranks[ranks.length - 1]).toEqual(['ship']);
  });

  it('renders every node of a cyclic graph rather than dropping some', () => {
    // Lint refuses cycles, but a preview must never silently omit a node.
    const ranks = ranksOf(
      def(
        [
          ['a', 'agent'],
          ['b', 'agent'],
        ],
        [
          ['a', 'b'],
          ['b', 'a'],
        ],
      ),
    );
    expect(ranks.flat().sort()).toEqual(['a', 'b']);
  });
});

describe('MiniGraph', () => {
  it('renders a node per definition node, with its title', () => {
    render(
      <MiniGraph
        definition={def(
          [
            ['plan', 'agent'],
            ['gate', 'gate'],
            ['ship', 'finalize'],
          ],
          [
            ['plan', 'gate'],
            ['gate', 'ship'],
          ],
        )}
      />,
    );
    expect(screen.getByTestId('mini-node-plan')).toBeTruthy();
    expect(screen.getByTestId('mini-node-gate')).toBeTruthy();
    expect(screen.getByTestId('mini-node-ship')).toBeTruthy();
    expect(screen.getByText('Node gate')).toBeTruthy();
  });

  it('says so when the workflow has no steps', () => {
    render(<MiniGraph definition={def([], [])} />);
    expect(screen.getByTestId('mini-graph').textContent).toContain('no steps');
  });
});
