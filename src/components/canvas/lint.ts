/**
 * Lint findings as the builder consumes them (task P3.3, PRD §6.3).
 *
 * The rules themselves live in Rust — `domain/workflow_graph.rs` plus each
 * node type's `NodeHandler::lint`, joined by `node_lint::lint_definition` and
 * served over the `workflow_lint` command. Nothing here re-implements a rule:
 * this module only mirrors the wire shape and indexes findings by what they
 * are anchored to, so the canvas can badge the offending node/edge and the
 * save button can name what is blocking it.
 *
 * That division matters. `connectRules.ts` (P3.1) *does* mirror rules, because
 * a connect gesture has to be judged in the same frame it happens; a full
 * definition lint is a round-trip we can afford, and the round-trip is what
 * guarantees the editor and the engine agree.
 */
import type { WorkflowDefinitionV2 } from './types';

/** Rust `LintSeverity`, serde snake_case. */
export type LintSeverity = 'error' | 'warning';

/** Rust `LintFinding`. `code` is drawn from a fixed vocabulary
 *  (`cycle`, `missing-prompt`, `redirect-not-ancestor`, …); the frontend
 *  treats it as an opaque tag for grouping and test assertions, never a
 *  switch it has to keep exhaustive. */
export interface LintFinding {
  severity: LintSeverity;
  code: string;
  /** Node this finding is anchored to, when node-shaped. */
  node?: string | null;
  /** `[from, to]` when edge-shaped — the Rust tuple. */
  edge?: [string, string] | null;
  message: string;
}

/** Findings grouped for rendering. Edge keys are `from->to`, matching the
 *  React Flow edge ids `flowGraph.ts` mints, so an edge lookup needs no
 *  second convention. */
export interface LintIndex {
  findings: LintFinding[];
  byNode: Map<string, LintFinding[]>;
  byEdge: Map<string, LintFinding[]>;
  /** Findings anchored to neither (`schema-invalid`) — rendered on the bar. */
  workflow: LintFinding[];
  errors: LintFinding[];
  warnings: LintFinding[];
  /** The save gate. Warnings are surfaced but never block (PRD §6.3). */
  hasErrors: boolean;
}

export const EMPTY_LINT: LintIndex = {
  findings: [],
  byNode: new Map(),
  byEdge: new Map(),
  workflow: [],
  errors: [],
  warnings: [],
  hasErrors: false,
};

/** Stable key for an edge anchor — identical to the React Flow edge id. */
export function edgeKey(from: string, to: string): string {
  return `${from}->${to}`;
}

function push(map: Map<string, LintFinding[]>, key: string, finding: LintFinding): void {
  const list = map.get(key);
  if (list) list.push(finding);
  else map.set(key, [finding]);
}

export function indexFindings(findings: LintFinding[]): LintIndex {
  const byNode = new Map<string, LintFinding[]>();
  const byEdge = new Map<string, LintFinding[]>();
  const workflow: LintFinding[] = [];
  const errors: LintFinding[] = [];
  const warnings: LintFinding[] = [];

  for (const f of findings) {
    if (f.severity === 'error') errors.push(f);
    else warnings.push(f);

    if (f.node) push(byNode, f.node, f);
    else if (f.edge) push(byEdge, edgeKey(f.edge[0], f.edge[1]), f);
    else workflow.push(f);
  }

  return {
    findings,
    byNode,
    byEdge,
    workflow,
    errors,
    warnings,
    hasErrors: errors.length > 0,
  };
}

/** `"2 errors · 1 warning"`, or `null` when the graph is clean. */
export function lintSummary(index: LintIndex): string | null {
  const parts: string[] = [];
  if (index.errors.length > 0) {
    parts.push(`${index.errors.length} error${index.errors.length === 1 ? '' : 's'}`);
  }
  if (index.warnings.length > 0) {
    parts.push(`${index.warnings.length} warning${index.warnings.length === 1 ? '' : 's'}`);
  }
  return parts.length > 0 ? parts.join(' · ') : null;
}

/**
 * One finding rendered as a line for the blocked-save toast: the offending
 * node's *title* where we can resolve it, since the author picked the title
 * and may never have seen the generated node id.
 */
export function describeFinding(
  finding: LintFinding,
  def: WorkflowDefinitionV2 | null,
): string {
  const titleOf = (id: string) => def?.nodes.find((n) => n.id === id)?.title ?? id;
  if (finding.node) return `${titleOf(finding.node)}: ${finding.message}`;
  if (finding.edge) {
    return `${titleOf(finding.edge[0])} → ${titleOf(finding.edge[1])}: ${finding.message}`;
  }
  return finding.message;
}
