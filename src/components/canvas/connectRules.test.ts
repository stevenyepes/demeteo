/**
 * Table-driven coverage for the design-mode connect rules (task P3.1) and the
 * pure graph edits they guard. Every rejection code asserted here has a
 * counterpart in the Rust lint (`domain/workflow_graph.rs`) — the point of
 * the tests is that the editor refuses the same shapes the engine does,
 * before the author can build them.
 */
import { describe, expect, it } from 'vitest';

import {
  atInstanceCap,
  canConnect,
  connectableTypesFrom,
  effectivePorts,
  portsCompatible,
} from './connectRules';
import { addNode, connectNodes, moveNodes, nextNodeId, removeEdge, removeNode } from './graphEdits';
import { byKind, type NodeTypeInfo } from './nodeCatalog';
import type { WorkflowDefinitionV2 } from './types';

/** The launch five, as `node_types_list` returns them. */
const CATALOG: NodeTypeInfo[] = [
  {
    kind: 'agent',
    label: 'Agent',
    summary: 'One agent turn.',
    config_schema: { type: 'object' },
    inputs: ['any'],
    outputs: ['text', 'file', 'task_list', 'verdict'],
    max_instances: null,
  },
  {
    kind: 'gate',
    label: 'Gate',
    summary: 'Human decision.',
    config_schema: { type: 'object' },
    inputs: ['any'],
    outputs: ['approval'],
    max_instances: null,
  },
  {
    kind: 'sequence',
    label: 'Sequence',
    summary: 'Task list fan-out.',
    config_schema: { type: 'object' },
    inputs: ['any'],
    outputs: ['text', 'file'],
    max_instances: null,
  },
  {
    kind: 'finalize',
    label: 'Finalize',
    summary: 'Squash and publish.',
    config_schema: { type: 'object' },
    inputs: ['any'],
    outputs: [],
    max_instances: 1,
  },
];

const TYPES = byKind(CATALOG);

function def(
  nodes: WorkflowDefinitionV2['nodes'],
  edges: WorkflowDefinitionV2['edges'] = [],
): WorkflowDefinitionV2 {
  return { schema_version: 2, id: 'wf', name: 'WF', nodes, edges };
}

const CHAIN = def(
  [
    { id: 'a', type: 'agent', title: 'Research' },
    { id: 'b', type: 'agent', title: 'Implement' },
    { id: 'c', type: 'finalize', title: 'Ship' },
  ],
  [
    { from: 'a', to: 'b' },
    { from: 'b', to: 'c' },
  ],
);

describe('portsCompatible', () => {
  it.each([
    ['text', 'text', true],
    ['text', 'file', false],
    ['any', 'verdict', true],
    ['approval', 'any', true],
  ] as const)('%s → %s = %s', (a, b, expected) => {
    expect(portsCompatible(a, b)).toBe(expected);
  });
});

describe('effectivePorts', () => {
  it('falls back to the registry type defaults when the node declares none', () => {
    const node = { id: 'a', type: 'agent', title: 'A' };
    expect(effectivePorts(node, TYPES.get('agent')).outputs).toEqual([
      'text',
      'file',
      'task_list',
      'verdict',
    ]);
  });

  it('lets a node narrow its type defaults via config.outputs', () => {
    const node = {
      id: 'a',
      type: 'agent',
      title: 'A',
      config: { outputs: [{ name: 'plan', type: 'task_list' }] },
    };
    expect(effectivePorts(node, TYPES.get('agent')).outputs).toEqual(['task_list']);
  });

  it('ignores an unparseable declaration rather than blocking every edge', () => {
    const node = { id: 'a', type: 'agent', title: 'A', config: { outputs: ['garbage'] } };
    expect(effectivePorts(node, TYPES.get('agent')).outputs).toEqual([
      'text',
      'file',
      'task_list',
      'verdict',
    ]);
  });

  it('treats an explicitly empty declaration as a real sink', () => {
    const node = { id: 'a', type: 'agent', title: 'A', config: { outputs: [] } };
    expect(effectivePorts(node, TYPES.get('agent')).outputs).toEqual([]);
  });
});

describe('canConnect', () => {
  it('accepts a plain forward edge', () => {
    expect(canConnect(CHAIN, TYPES, 'a', 'c')).toEqual({ ok: true });
  });

  it('rejects a self-edge', () => {
    const v = canConnect(CHAIN, TYPES, 'a', 'a');
    expect(v).toMatchObject({ ok: false, code: 'self-edge' });
  });

  it('rejects an edge that would close a cycle', () => {
    // b → a would loop, since a already reaches b.
    const v = canConnect(CHAIN, TYPES, 'b', 'a');
    expect(v).toMatchObject({ ok: false, code: 'cycle' });
    if (!v.ok) expect(v.message).toContain('Implement');
  });

  it('rejects a cycle closed through a longer path', () => {
    // c → a would loop via a → b → c.
    expect(canConnect(CHAIN, TYPES, 'c', 'a')).toMatchObject({ code: 'cycle' });
  });

  it('rejects a duplicate edge', () => {
    expect(canConnect(CHAIN, TYPES, 'a', 'b')).toMatchObject({
      ok: false,
      code: 'duplicate-edge',
    });
  });

  it('rejects anything downstream of finalize (finalize-not-sink)', () => {
    const withTail = def(
      [...CHAIN.nodes, { id: 'd', type: 'agent', title: 'After' }],
      CHAIN.edges,
    );
    const v = canConnect(withTail, TYPES, 'c', 'd');
    expect(v).toMatchObject({ ok: false, code: 'port-type-mismatch' });
    if (!v.ok) expect(v.message).toContain('ends the run');
  });

  it('rejects an edge whose ports do not overlap', () => {
    const narrow = def([
      { id: 'a', type: 'agent', title: 'A', config: { outputs: [{ name: 'v', type: 'verdict' }] } },
      { id: 'b', type: 'agent', title: 'B', config: { inputs: [{ name: 'f', type: 'file' }] } },
    ]);
    expect(canConnect(narrow, TYPES, 'a', 'b')).toMatchObject({
      ok: false,
      code: 'port-type-mismatch',
    });
  });

  it('rejects an edge naming a node that is not on the canvas', () => {
    expect(canConnect(CHAIN, TYPES, 'a', 'ghost')).toMatchObject({
      ok: false,
      code: 'unknown-node',
    });
  });

  it('stays permissive for the shipped starter shapes', () => {
    // gate → sequence is a real starter edge; a narrower input declaration on
    // `sequence` would break graphs the engine runs happily.
    const shape = def([
      { id: 'g', type: 'gate', title: 'Review' },
      { id: 's', type: 'sequence', title: 'Implement' },
    ]);
    expect(canConnect(shape, TYPES, 'g', 's')).toEqual({ ok: true });
  });
});

describe('atInstanceCap', () => {
  it('caps finalize at one and leaves uncapped types alone', () => {
    expect(atInstanceCap(CHAIN, TYPES.get('finalize')!)).toBe(true);
    expect(atInstanceCap(CHAIN, TYPES.get('agent')!)).toBe(false);
    expect(atInstanceCap(def([]), TYPES.get('finalize')!)).toBe(false);
  });
});

describe('connectableTypesFrom', () => {
  it('offers every compatible type that is not at its cap', () => {
    const kinds = connectableTypesFrom(CHAIN, CATALOG, 'a').map((t) => t.kind);
    // `finalize` is already present and capped at one, so it drops out.
    expect(kinds).toEqual(['agent', 'gate', 'sequence']);
  });

  it('offers finalize once the existing one is gone', () => {
    const kinds = connectableTypesFrom(removeNode(CHAIN, 'c'), CATALOG, 'a').map((t) => t.kind);
    expect(kinds).toContain('finalize');
  });

  it('offers nothing from a sink', () => {
    expect(connectableTypesFrom(CHAIN, CATALOG, 'c')).toEqual([]);
  });

  it('filters by port type for a narrow producer', () => {
    const narrow = def([
      { id: 'a', type: 'agent', title: 'A', config: { outputs: [{ name: 'v', type: 'verdict' }] } },
    ]);
    const picky: NodeTypeInfo[] = [
      { ...CATALOG[0], kind: 'takes-verdict', inputs: ['verdict'] },
      { ...CATALOG[0], kind: 'takes-file', inputs: ['file'] },
    ];
    expect(connectableTypesFrom(narrow, picky, 'a').map((t) => t.kind)).toEqual(['takes-verdict']);
  });
});

describe('graphEdits', () => {
  it('mints readable, collision-free ids', () => {
    expect(nextNodeId(def([]), 'agent')).toBe('agent');
    expect(nextNodeId(def([{ id: 'agent', type: 'agent', title: 'A' }]), 'agent')).toBe('agent-2');
  });

  it('adds a node at the drop position and optionally wires the drag edge', () => {
    const { def: next, nodeId } = addNode(CHAIN, TYPES.get('gate')!, { x: 10, y: 20 }, 'a');
    expect(nodeId).toBe('gate');
    const added = next.nodes.find((n) => n.id === 'gate')!;
    expect(added).toMatchObject({ type: 'gate', title: 'Gate', position: { x: 10, y: 20 } });
    expect(next.edges).toContainEqual({ from: 'a', to: 'gate' });
    // The input definition is untouched — snapshots stay usable for undo.
    expect(CHAIN.nodes).toHaveLength(3);
  });

  it('adds a bare node when no drag source is given', () => {
    const { def: next } = addNode(CHAIN, TYPES.get('agent')!, { x: 0, y: 0 });
    expect(next.edges).toEqual(CHAIN.edges);
    expect(next.nodes[next.nodes.length - 1]).toMatchObject({ id: 'agent', title: 'Agent' });
  });

  it('disambiguates the title of a second node of the same type', () => {
    const once = addNode(def([]), TYPES.get('agent')!, { x: 0, y: 0 }).def;
    const twice = addNode(once, TYPES.get('agent')!, { x: 0, y: 0 });
    expect(twice.nodeId).toBe('agent-2');
    expect(twice.def.nodes[1].title).toBe('Agent 2');
  });

  it('removes a node with its edges', () => {
    const next = removeNode(CHAIN, 'b');
    expect(next.nodes.map((n) => n.id)).toEqual(['a', 'c']);
    expect(next.edges).toEqual([]);
  });

  it('defuses a retry redirect pointed at a removed node', () => {
    // A dangling `redirect_to` is the audit-F39 bug class the builder exists
    // to make impossible — deleting the target must not leave one behind.
    const withRedirect = def(
      [
        { id: 'a', type: 'agent', title: 'A' },
        {
          id: 'b',
          type: 'agent',
          title: 'B',
          retry: { verdict: { strategy: 'redirect', redirect_to: 'a', max_attempts: 3 } },
        },
      ],
      [{ from: 'a', to: 'b' }],
    );
    const next = removeNode(withRedirect, 'a');
    expect(next.nodes[0].retry?.verdict).toMatchObject({
      strategy: 'fail',
      redirect_to: null,
      max_attempts: 3,
    });
  });

  it('leaves unrelated redirects intact', () => {
    const withRedirect = def([
      { id: 'a', type: 'agent', title: 'A' },
      { id: 'b', type: 'agent', title: 'B' },
      {
        id: 'c',
        type: 'agent',
        title: 'C',
        retry: { verdict: { strategy: 'redirect', redirect_to: 'a' } },
      },
    ]);
    expect(removeNode(withRedirect, 'b').nodes[1].retry?.verdict?.redirect_to).toBe('a');
  });

  it('connects, disconnects, and moves', () => {
    expect(connectNodes(CHAIN, 'a', 'c').edges).toHaveLength(3);
    expect(removeEdge(CHAIN, 'a', 'b').edges).toEqual([{ from: 'b', to: 'c' }]);
    const moved = moveNodes(CHAIN, { b: { x: 5, y: 6 } });
    expect(moved.nodes[1].position).toEqual({ x: 5, y: 6 });
    expect(moved.nodes[0].position).toBeUndefined();
  });
});
