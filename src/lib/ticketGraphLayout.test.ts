// The graph's shape is the whole of its design: a node sits below everything
// it depends on, and a fan-out is a wider row. The mock's x/y literals are
// fixtures, so these pin the derivation rather than the numbers it produced.

import { describe, expect, it } from 'vitest';

import { layoutTicketGraph, NODE_H, NODE_W } from './ticketGraphLayout';
import type { Blocker, TicketLane, TicketView } from '../types';

function ticket(
  id: string,
  seq: number,
  blockedBy: string[],
  options: { lane?: TicketLane; blockers?: Blocker[] } = {},
): TicketView {
  return {
    ticket: {
      id,
      discovery_id: 'dsc-1',
      seq,
      title: `ticket ${seq}`,
      description: '',
      acceptance: [],
      files: [],
      blocked_by: blockedBy,
      test_command: null,
      workflow_id: null,
      agent_kind: null,
      model: null,
      effort: null,
      attachments: [],
      state: 'unstarted',
      drop_reason: null,
      force_start_reason: null,
      force_started_at: null,
      feature_id: null,
      created_at: 0,
      updated_at: 0,
    },
    standing: {
      id,
      lane: options.lane ?? 'blocked',
      startable: false,
      blockers: options.blockers ?? blockedBy.map((blocker) => ({ id: blocker, reason: 'outstanding' })),
    },
    feature: null,
  };
}

function nodeOf(layout: ReturnType<typeof layoutTicketGraph>, id: string) {
  const node = layout.nodes.find((candidate) => candidate.id === id);
  if (!node) throw new Error(`no node for ${id}`);
  return node;
}

describe('layoutTicketGraph', () => {
  it('puts a dependent below its prerequisite', () => {
    const layout = layoutTicketGraph([ticket('a', 1, []), ticket('b', 2, ['a'])]);

    expect(nodeOf(layout, 'b').y).toBeGreaterThan(nodeOf(layout, 'a').y + NODE_H);
  });

  it('lays a fan-out out as one wider row', () => {
    const layout = layoutTicketGraph([
      ticket('a', 1, []),
      ticket('b', 2, ['a']),
      ticket('c', 3, ['a']),
    ]);

    const b = nodeOf(layout, 'b');
    const c = nodeOf(layout, 'c');
    expect(b.y).toBe(c.y);
    expect(Math.abs(b.x - c.x)).toBeGreaterThanOrEqual(NODE_W);
    expect(layout.width).toBeGreaterThanOrEqual(2 * NODE_W);
  });

  it('marks an edge met only when the prerequisite no longer blocks', () => {
    const layout = layoutTicketGraph([
      ticket('a', 1, [], { lane: 'landed' }),
      ticket('b', 2, ['a'], { lane: 'ready', blockers: [] }),
      ticket('c', 3, ['a'], { lane: 'blocked' }),
    ]);

    const toB = layout.edges.find((edge) => edge.to === 'b');
    const toC = layout.edges.find((edge) => edge.to === 'c');
    expect(toB?.met).toBe(true);
    expect(toC?.met).toBe(false);
  });

  it('draws no edge for a prerequisite outside this discovery', () => {
    const layout = layoutTicketGraph([ticket('a', 1, ['gone'])]);

    expect(layout.edges).toEqual([]);
    expect(layout.nodes).toHaveLength(1);
  });

  it('still places every node when the edges form a cycle', () => {
    const layout = layoutTicketGraph([ticket('a', 1, ['b']), ticket('b', 2, ['a'])]);

    expect(layout.nodes.map((node) => node.id).sort()).toEqual(['a', 'b']);
  });
});
