/**
 * Starting shapes for a new workflow (task P3.6, PRD §6.3).
 *
 * "New workflow" offers a starter clone or one of three shapes. The clone path
 * reads a real stored workflow through the backend; the shapes live here, as
 * plain schema-v2 data, because that is all they are — no IPC, no migration,
 * and testable without mounting anything.
 *
 * Two rules every template honours, both learned from the lint rules the write
 * path enforces (P3.3):
 *
 * - **Every agent node carries a real prompt.** An empty `prompt_template` is
 *   a `missing-prompt` *error*, so a template without one would drop the author
 *   into a graph that cannot be saved until they have filled in every node —
 *   the opposite of a head start.
 * - **Prompts use the same placeholders as the bundled starters**
 *   (`{{feature_description}}`, `{{test_command}}`, …) and declare their
 *   artifacts, so a templated workflow is runnable as-is rather than being a
 *   diagram of one.
 *
 * The blank shape is deliberately empty: someone who picks "blank" wants the
 * palette, not a node to delete first.
 */
import type { NodeConfigV2, WorkflowDefinitionV2 } from './types';

/** Vertical spacing between generated nodes — matches the Rust migration's
 *  synthesized layout so a templated graph and a migrated one look alike. */
const ROW = 160;

export interface WorkflowTemplate {
  id: string;
  label: string;
  /** One line under the label in the picker. */
  summary: string;
  build: () => WorkflowDefinitionV2;
}

function agent(
  id: string,
  title: string,
  promptTemplate: string,
  extra: Record<string, unknown> = {},
): NodeConfigV2 {
  return {
    id,
    type: 'agent',
    title,
    config: { prompt_template: promptTemplate, capability: 'artifacts', ...extra },
  };
}

function artifact(name: string, path: string) {
  return [{ name, capture: { kind: 'last_write_to', path }, mode: 'full' }];
}

/** Chain the nodes in array order and lay them out in a column. */
function chain(id: string, name: string, nodes: NodeConfigV2[]): WorkflowDefinitionV2 {
  return {
    schema_version: 2,
    id,
    name,
    nodes: nodes.map((node, i) => ({ ...node, position: { x: 0, y: i * ROW } })),
    edges: nodes.slice(1).map((node, i) => ({ from: nodes[i].id, to: node.id })),
  };
}

const PLAN_PROMPT = `You are a senior engineer producing a concise, actionable implementation plan.

Feature description: {{feature_description}}
Project conventions: {{project_conventions}}
Repositories in scope: {{repo_list}}

## Your task
Explore the relevant area of the codebase, then write a plan with:

1. **Files to Change** — table: file path | create/modify | what changes
2. **Implementation Approach** — atomic, ordered steps
3. **Acceptance Criteria** — 2-5 binary pass/fail statements

## Rules
- Do NOT write source code. Produce the plan document only.

## Output artifact (required)
Write the plan to \`artifacts/implementation-plan.md\`. Replying with the plan
is not sufficient — the file must exist.`;

const IMPLEMENT_PROMPT = `You are an implementation engineer executing a predefined plan.

Feature description: {{feature_description}}
Plan artifact: [attached]
Build command: {{build_command}}
Test command: {{test_command}}

## Rules
1. Follow the plan's Implementation Approach step by step.
2. Touch only the files the plan lists. No scope creep.
3. Write or update tests for every change.
4. Leave the build and the test suite green.`;

const VALIDATE_PROMPT = `You are a validation engineer checking the implementation against its plan.

Feature description: {{feature_description}}
Acceptance criteria: [attached — the plan artifact]

The test harness has already been run; its output is below. Judge the work
against the acceptance criteria, not your own taste.

## Output artifact (required)
Write your assessment to \`artifacts/validation-report.md\`, ending with a
single line: \`VERDICT: PASS\` or \`VERDICT: FAIL\`.`;

/** Retry policy the validate node gets in the three-step shape: a failing
 *  verdict redirects to implement, which is v1's `on_failure` in v2 clothes
 *  (and exactly what `migrate_v1_to_v2` emits for a looping starter). */
export function withValidateLoop(def: WorkflowDefinitionV2): WorkflowDefinitionV2 {
  const redirect = {
    strategy: 'redirect' as const,
    redirect_to: 's-implement',
    max_attempts: 3,
    feedback: true,
  };
  return {
    ...def,
    nodes: def.nodes.map((n) =>
      n.id === 's-validate' ? { ...n, retry: { verdict: redirect, agent_failure: redirect } } : n,
    ),
  };
}

export const WORKFLOW_TEMPLATES: WorkflowTemplate[] = [
  {
    id: 'blank',
    label: 'Blank',
    summary: 'An empty canvas. Drag node types in from the palette.',
    build: () => ({
      schema_version: 2,
      id: 'wf-new',
      name: 'New Workflow',
      nodes: [],
      edges: [],
    }),
  },
  {
    id: 'plan-implement-validate',
    label: 'Plan → Implement → Validate',
    summary: 'The common three-step shape: plan the work, do it, check it.',
    build: () =>
      // The validate node loops back to implement on a failing verdict — the
      // shape every bundled starter uses, pre-wired so the template teaches it
      // rather than leaving the author to discover the retry sub-form.
      withValidateLoop(
        chain('wf-new', 'New Workflow', [
          agent('s-plan', 'Plan', PLAN_PROMPT, {
            artifacts: artifact('implementation-plan', 'artifacts/implementation-plan.md'),
            allow_shell: true,
          }),
          agent('s-implement', 'Implement', IMPLEMENT_PROMPT, {
            capability: 'implement',
          }),
          agent('s-validate', 'Validate', VALIDATE_PROMPT, {
            capability: 'verify',
            artifacts: artifact('validation-report', 'artifacts/validation-report.md'),
            verifier: {
              kind: 'agent_judgment',
              instructions:
                "Judge the implementation against the plan's acceptance criteria and the harness output.",
            },
          }),
          { id: 's-finalize', type: 'finalize', title: 'Finalize', config: {} },
        ]),
      ),
  },
  {
    id: 'gated',
    label: 'Plan → Gate → Implement → Validate → Gate',
    summary: 'Adds a human review before the work starts and before it ships.',
    build: () =>
      chain('wf-new', 'New Workflow', [
        agent('s-plan', 'Plan', PLAN_PROMPT, {
          artifacts: artifact('implementation-plan', 'artifacts/implementation-plan.md'),
          allow_shell: true,
        }),
        {
          id: 's-gate-plan',
          type: 'gate',
          title: 'Review plan',
          config: { prompt_template: 'Review the implementation plan before any code is written.' },
        },
        agent('s-implement', 'Implement', IMPLEMENT_PROMPT, { capability: 'implement' }),
        agent('s-validate', 'Validate', VALIDATE_PROMPT, {
          capability: 'verify',
          artifacts: artifact('validation-report', 'artifacts/validation-report.md'),
        }),
        {
          id: 's-gate-ship',
          type: 'gate',
          title: 'Review diff',
          config: { prompt_template: 'Review the diff before it is squashed and published.' },
        },
        { id: 's-finalize', type: 'finalize', title: 'Finalize', config: {} },
      ]),
  },
];

export function templateById(id: string): WorkflowTemplate | undefined {
  return WORKFLOW_TEMPLATES.find((t) => t.id === id);
}
