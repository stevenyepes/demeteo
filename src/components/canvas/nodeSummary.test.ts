/**
 * Node-card config essence (task P3.2) — PRD §6.3's anti-"identical boxes"
 * rule. The load-bearing property is the *degradation*: a node type that uses
 * none of the conventional config keys must produce an empty essence rather
 * than a broken card, because that is the case the frontend can't anticipate.
 */
import { describe, expect, it } from 'vitest';

import { isEssenceEmpty, nodeEssence } from './nodeSummary';
import { toFlowGraph } from './flowGraph';
import type { NodeConfigV2, WorkflowDefinitionV2 } from './types';

const node = (over: Partial<NodeConfigV2> = {}): NodeConfigV2 => ({
  id: 'n1',
  type: 'agent',
  title: 'Implement',
  ...over,
});

describe('nodeEssence', () => {
  it('surfaces the pinned agent, model, effort and write scope', () => {
    const essence = nodeEssence(
      node({
        config: {
          agent_kind: 'claude-code',
          model: 'opus',
          effort: 'high',
          capability: 'read_only',
        },
      }),
    );
    expect(essence.badges.map((b) => [b.kind, b.label])).toEqual([
      ['agent', 'claude-code'],
      ['model', 'opus'],
      ['effort', 'high'],
      ['capability', 'read-only'],
    ]);
  });

  it('flags the permission opt-ins only when they are on', () => {
    expect(
      nodeEssence(node({ config: { allow_network: true, allow_shell: false } })).badges.map(
        (b) => b.label,
      ),
    ).toEqual(['net']);
  });

  it('shows a verifier as a dot, not a badge', () => {
    const essence = nodeEssence(node({ config: { verifier: { instructions: 'check' } } }));
    expect(essence.verifier).toBe(true);
    expect(essence.badges).toEqual([]);
  });

  it('renders the retry summary in the PRD spelling', () => {
    const essence = nodeEssence(
      node({ retry: { verdict: { strategy: 'redirect', redirect_to: 'implement', max_attempts: 3 } } }),
    );
    expect(essence.retry).toEqual(['verdict→implement ×3']);
  });

  it('is empty — not broken — for a node type it knows nothing about', () => {
    const essence = nodeEssence(
      node({ type: 'command', config: { command: 'npm test', timeout_secs: 600 } }),
    );
    expect(isEssenceEmpty(essence)).toBe(true);
  });

  it('ignores blank strings rather than rendering an empty chip', () => {
    expect(isEssenceEmpty(nodeEssence(node({ config: { model: '   ' } })))).toBe(true);
  });
});

describe('toFlowGraph essence wiring', () => {
  const def: WorkflowDefinitionV2 = {
    schema_version: 2,
    id: 'wf',
    name: 'W',
    nodes: [node({ config: { model: 'opus' } })],
    edges: [],
  };

  it('attaches essence only in design mode', () => {
    expect(toFlowGraph(def).nodes[0].data.essence).toBeUndefined();
    expect(toFlowGraph(def, { showEssence: true }).nodes[0].data.essence?.badges).toHaveLength(1);
  });

  it('omits an empty essence so the card can skip the whole row', () => {
    const bare: WorkflowDefinitionV2 = { ...def, nodes: [node()] };
    expect(toFlowGraph(bare, { showEssence: true }).nodes[0].data.essence).toBeUndefined();
  });
});
