/**
 * Lint indexing + how findings reach the canvas (task P3.3).
 *
 * Nothing here asserts a *rule* — the rules are Rust's, covered by
 * `node_lint`'s tests. What matters on this side is that a finding lands on the
 * thing it accuses (node badge, edge tint, or the workflow-level bar), that
 * only errors gate the save, and that a message shown to the author names the
 * node by its **title** rather than by a generated id.
 */
import { describe, expect, it } from 'vitest';

import { toFlowGraph } from './flowGraph';
import {
  describeFinding,
  edgeKey,
  indexFindings,
  lintSummary,
  type LintFinding,
} from './lint';
import type { WorkflowDefinitionV2 } from './types';

const def: WorkflowDefinitionV2 = {
  schema_version: 2,
  id: 'wf-l',
  name: 'Lint',
  nodes: [
    { id: 'plan', type: 'agent', title: 'Research Codebase' },
    { id: 'ship', type: 'finalize', title: 'Publish Branch' },
  ],
  edges: [{ from: 'plan', to: 'ship' }],
};

const nodeError: LintFinding = {
  severity: 'error',
  code: 'missing-prompt',
  node: 'plan',
  message: "agent node 'plan' has no prompt_template",
};
const nodeWarning: LintFinding = {
  severity: 'warning',
  code: 'dead-end',
  node: 'plan',
  message: "node 'plan' is a sink but not the finalize node",
};
const edgeError: LintFinding = {
  severity: 'error',
  code: 'port-type-mismatch',
  edge: ['plan', 'ship'],
  message: "edge 'plan' → 'ship' connects no compatible ports",
};
const workflowError: LintFinding = {
  severity: 'error',
  code: 'schema-invalid',
  message: 'definition is not a readable schema-v2 workflow',
};

describe('indexFindings', () => {
  it('groups by anchor and separates severities', () => {
    const index = indexFindings([nodeError, nodeWarning, edgeError, workflowError]);

    expect(index.byNode.get('plan')).toEqual([nodeError, nodeWarning]);
    expect(index.byEdge.get(edgeKey('plan', 'ship'))).toEqual([edgeError]);
    expect(index.workflow).toEqual([workflowError]);
    expect(index.errors).toHaveLength(3);
    expect(index.warnings).toEqual([nodeWarning]);
    expect(index.hasErrors).toBe(true);
  });

  it('does not let warnings alone block a save', () => {
    const index = indexFindings([nodeWarning]);
    expect(index.hasErrors).toBe(false);
    expect(lintSummary(index)).toBe('1 warning');
  });

  it('summarizes counts, and says nothing when clean', () => {
    expect(lintSummary(indexFindings([]))).toBeNull();
    expect(lintSummary(indexFindings([nodeError, edgeError, nodeWarning]))).toBe(
      '2 errors · 1 warning',
    );
  });

  it('uses the edge id convention the canvas mints', () => {
    // A different key here would mean edge findings silently never render.
    const { edges } = toFlowGraph(def);
    expect(edges[0].id).toBe(edgeKey('plan', 'ship'));
  });
});

describe('describeFinding', () => {
  it('names nodes by title, since the author never chose the id', () => {
    expect(describeFinding(nodeError, def)).toBe(
      "Research Codebase: agent node 'plan' has no prompt_template",
    );
    expect(describeFinding(edgeError, def)).toContain('Research Codebase → Publish Branch');
  });

  it('falls back to ids, and passes workflow-level findings through', () => {
    expect(describeFinding(nodeError, null)).toBe(
      "plan: agent node 'plan' has no prompt_template",
    );
    expect(describeFinding(workflowError, def)).toBe(workflowError.message);
  });
});

describe('toFlowGraph lint overlay', () => {
  it('badges the accused node and leaves clean ones bare', () => {
    const { nodes } = toFlowGraph(def, { lint: indexFindings([nodeError, nodeWarning]) });
    const plan = nodes.find((n) => n.id === 'plan')!;
    const ship = nodes.find((n) => n.id === 'ship')!;

    expect(plan.data.lint?.errors).toEqual([nodeError.message]);
    expect(plan.data.lint?.warnings).toEqual([nodeWarning.message]);
    expect(ship.data.lint).toBeUndefined();
  });

  it('tints an edge by its worst finding', () => {
    const errored = toFlowGraph(def, { lint: indexFindings([edgeError]) }).edges[0];
    expect(errored.style?.stroke).toBe('#f43f5e');
    expect((errored.data as { lint?: string[] })?.lint).toEqual([edgeError.message]);

    const warned = toFlowGraph(def, {
      lint: indexFindings([{ ...edgeError, severity: 'warning' }]),
    }).edges[0];
    expect(warned.style?.stroke).toBe('#f59e0b');

    expect(toFlowGraph(def).edges[0].style).toBeUndefined();
  });

  it('keeps a guarded edge labeled when it also carries a finding', () => {
    const guarded: WorkflowDefinitionV2 = {
      ...def,
      edges: [{ from: 'plan', to: 'ship', when: "${{ nodes.plan.outputs.verdict != 'FAIL' }}" }],
    };
    const edge = toFlowGraph(guarded, { lint: indexFindings([edgeError]) }).edges[0];
    expect(edge.label).toBe('when');
    expect((edge.data as { when?: string }).when).toContain('verdict');
    expect((edge.data as { lint?: string[] }).lint).toHaveLength(1);
  });
});
