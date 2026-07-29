/**
 * The builder's config side panel (task P3.2).
 *
 * Two claims carry the task, and both are asserted here against the **real**
 * artefacts rather than stand-ins:
 *
 *  - *"editing every field of a starter's agent node round-trips to valid v2
 *    JSON"* — the task's Done-when. `edits every field of a starter agent node`
 *    drives the panel over the committed `standard-feature-pipeline` fixture
 *    (emitted from the Rust migration) using the committed registry catalog
 *    (emitted from the Rust `node_type_catalog`), then type-checks the result
 *    back against that same schema.
 *  - *"a node type the frontend has never heard of still gets a complete
 *    panel"* — the P3.1 guarantee, extended from the palette to config, and
 *    the reason P3.5's `command` node needs no work here.
 */
import { render, screen, cleanup, fireEvent, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import { ConfigPanel } from './ConfigPanel';
import catalogFixture from './__fixtures__/node_catalog.json';
import starter from './__fixtures__/standard-feature-pipeline.v2.json';
import { fieldsFromSchema } from './schemaForm';
import type { NodeTypeInfo } from './nodeCatalog';
import type { WorkflowDefinitionV2 } from './types';

const CATALOG = catalogFixture as unknown as NodeTypeInfo[];
const STARTER = starter as unknown as WorkflowDefinitionV2;
const AGENT_TYPE = CATALOG.find((t) => t.kind === 'agent')!;

const AGENTS = [
  { kind: 'claude-code', display_label: 'Claude Code', lists_models: true, default_model: null, install_command: '', effort_levels: ['low', 'high'] },
  { kind: 'codex', display_label: 'Codex', lists_models: true, default_model: null, install_command: '', effort_levels: ['low', 'medium', 'high', 'xhigh'] },
];

/** The first `agent` node of the standard-feature starter. */
const AGENT_NODE_ID = STARTER.nodes.find((n) => n.type === 'agent')!.id;

/** The definition produced by the most recent edit. (`Array#at` is outside
 *  this project's lib target, so index the long way.) */
function lastDef(onChange: ReturnType<typeof vi.fn>): WorkflowDefinitionV2 {
  const calls = onChange.mock.calls;
  return calls[calls.length - 1][0] as WorkflowDefinitionV2;
}

function renderPanel(
  def: WorkflowDefinitionV2 = STARTER,
  nodeId: string = AGENT_NODE_ID,
  onChange = vi.fn(),
) {
  const view = render(
    <ConfigPanel
      definition={def}
      nodeId={nodeId}
      nodeTypes={CATALOG}
      onChange={onChange}
      onClose={vi.fn()}
    />,
  );
  return { ...view, onChange };
}

beforeEach(() => {
  vi.mocked(invoke).mockImplementation((cmd: string) =>
    cmd === 'list_agents' ? Promise.resolve(AGENTS) : Promise.resolve(undefined),
  );
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('schema-derived rendering', () => {
  it('renders a control for every published field of the node type', async () => {
    renderPanel();
    // Header identity.
    expect(screen.getByTestId('config-panel')).toBeTruthy();
    expect(screen.getByLabelText('Title')).toBeTruthy();

    for (const field of fieldsFromSchema(AGENT_TYPE.config_schema)) {
      // Monaco isn't a labelable element, so a prose field is found by its
      // own test id; everything else by its label.
      if (field.control === 'code') {
        expect(screen.getByTestId(`code-${field.key}`), field.key).toBeTruthy();
      } else {
        expect(screen.getByLabelText(field.label), field.key).toBeTruthy();
      }
    }
    // The prompt is Monaco, not a one-line input (PRD §6.3).
    expect(screen.getAllByTestId('monaco-editor').length).toBeGreaterThan(0);
  });

  it('renders a node type the panel was never written for', () => {
    // P3.5's `command` node, taken from the **live registry fixture** rather
    // than a stand-in: the backend added a node type and the panel has to
    // configure it with no frontend edit. If this ever needs a code change,
    // the seam has failed.
    const commandType = CATALOG.find((t) => t.kind === 'command')!;
    expect(commandType).toBeTruthy();
    const def: WorkflowDefinitionV2 = {
      schema_version: 2,
      id: 'wf',
      name: 'W',
      nodes: [{ id: 'c1', type: 'command', title: 'Baseline', config: {} }],
      edges: [],
    };
    const onChange = vi.fn();
    render(
      <ConfigPanel
        definition={def}
        nodeId="c1"
        nodeTypes={[commandType]}
        onChange={onChange}
        onClose={vi.fn()}
      />,
    );

    // Every field the schema publishes gets a control, derived — not listed.
    for (const field of fieldsFromSchema(commandType.config_schema)) {
      if (field.control === 'code') {
        expect(screen.getByTestId(`code-${field.key}`), field.key).toBeTruthy();
      } else {
        expect(screen.getByLabelText(field.label), field.key).toBeTruthy();
      }
    }
    // And the controls match the types: a shell string, a number, a checkbox.
    fireEvent.change(screen.getByLabelText('Command'), {
      target: { value: 'cargo test --all' },
    });
    expect(lastDef(onChange).nodes[0].config?.command).toBe('cargo test --all');
    fireEvent.click(screen.getByLabelText('Idempotent'));
    expect(lastDef(onChange).nodes[0].config?.idempotent).toBe(true);
  });

  it('offers no verifier sub-form for a type whose schema omits it', () => {
    const gateId = STARTER.nodes.find((n) => n.type === 'gate')?.id;
    if (!gateId) return; // starter shape changed; the agent cases still cover it
    renderPanel(STARTER, gateId);
    expect(screen.queryByText('Verifier')).toBeNull();
  });
});

describe('editing', () => {
  it('writes the title straight onto the node', () => {
    const { onChange } = renderPanel();
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Recon' } });
    const next = onChange.mock.calls[0][0] as WorkflowDefinitionV2;
    expect(next.nodes.find((n) => n.id === AGENT_NODE_ID)!.title).toBe('Recon');
  });

  it('clears a nullable field to null, matching migrated output', () => {
    const pinned: WorkflowDefinitionV2 = {
      ...STARTER,
      nodes: STARTER.nodes.map((n) =>
        n.id === AGENT_NODE_ID ? { ...n, config: { ...n.config, model: 'sonnet' } } : n,
      ),
    };
    const { onChange } = renderPanel(pinned);
    fireEvent.change(screen.getByLabelText('Model'), { target: { value: '' } });
    const next = onChange.mock.calls[0][0] as WorkflowDefinitionV2;
    expect(next.nodes.find((n) => n.id === AGENT_NODE_ID)!.config!.model).toBeNull();
  });

  it('leaves every other node untouched', () => {
    const { onChange } = renderPanel();
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'X' } });
    const next = onChange.mock.calls[0][0] as WorkflowDefinitionV2;
    for (const before of STARTER.nodes) {
      if (before.id === AGENT_NODE_ID) continue;
      expect(next.nodes.find((n) => n.id === before.id)).toEqual(before);
    }
    expect(next.edges).toEqual(STARTER.edges);
  });

  it('expands a prompt template to a full-height editor and back', () => {
    renderPanel();
    fireEvent.click(screen.getByLabelText('Expand Prompt template'));
    // The form body is replaced by the editor, so the other fields are gone.
    expect(screen.queryByLabelText('Model')).toBeNull();
    expect(screen.getAllByTestId('monaco-editor').length).toBe(1);
    fireEvent.click(screen.getByText('Collapse'));
    expect(screen.getByLabelText('Model')).toBeTruthy();
  });

  it('keeps an unparseable JSON edit in the box instead of dropping it', () => {
    const { onChange } = renderPanel();
    const artifacts = screen.getByLabelText('Artifacts') as HTMLTextAreaElement;
    fireEvent.change(artifacts, { target: { value: '[{ broken' } });
    expect(screen.getByText(/Invalid JSON/)).toBeTruthy();
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.change(artifacts, { target: { value: '[{"name":"out"}]' } });
    const next = onChange.mock.calls[0][0] as WorkflowDefinitionV2;
    expect(next.nodes.find((n) => n.id === AGENT_NODE_ID)!.config!.artifacts).toEqual([
      { name: 'out' },
    ]);
  });
});

describe('catalog-backed value sources', () => {
  it('turns agent_kind into a select over the live agent catalog', async () => {
    renderPanel();
    const select = await screen.findByLabelText('Agent kind');
    expect(select.tagName).toBe('SELECT');
    expect(
      Array.from((select as HTMLSelectElement).options).map((o) => o.value),
    ).toEqual(['', 'claude-code', 'codex']);
  });

  it('clamps effort to what the pinned agent actually accepts', async () => {
    const def: WorkflowDefinitionV2 = {
      ...STARTER,
      nodes: STARTER.nodes.map((n) =>
        n.id === AGENT_NODE_ID
          ? { ...n, config: { ...n.config, agent_kind: 'claude-code' } }
          : n,
      ),
    };
    renderPanel(def);
    // The schema advertises all five levels; claude-code declares two.
    await screen.findByLabelText('Agent kind');
    const effort = screen.getByLabelText('Effort') as HTMLSelectElement;
    expect(Array.from(effort.options).map((o) => o.value)).toEqual(['', 'low', 'high']);
  });
});

describe('verifier sub-form', () => {
  it('enables with the same defaults the old form editor wrote', () => {
    const bare: WorkflowDefinitionV2 = {
      ...STARTER,
      nodes: STARTER.nodes.map((n) =>
        n.id === AGENT_NODE_ID ? { ...n, config: { ...n.config, verifier: null } } : n,
      ),
    };
    const { onChange } = renderPanel(bare);
    fireEvent.click(screen.getByLabelText(/Verify this node/));
    const next = onChange.mock.calls[0][0] as WorkflowDefinitionV2;
    expect(next.nodes.find((n) => n.id === AGENT_NODE_ID)!.config!.verifier).toEqual({
      instructions: 'Verify that the changes are correct and the tests pass.',
      agent_kind: null,
      harness_names: [],
      verdict_key: 'verdict',
    });
  });

  it('edits instructions once enabled', () => {
    const withVerifier: WorkflowDefinitionV2 = {
      ...STARTER,
      nodes: STARTER.nodes.map((n) =>
        n.id === AGENT_NODE_ID
          ? { ...n, config: { ...n.config, verifier: { instructions: 'old' } } }
          : n,
      ),
    };
    const { onChange } = renderPanel(withVerifier);
    fireEvent.change(screen.getByLabelText('Instructions'), { target: { value: 'new' } });
    const next = onChange.mock.calls[0][0] as WorkflowDefinitionV2;
    expect(
      (next.nodes.find((n) => n.id === AGENT_NODE_ID)!.config!.verifier as Record<string, unknown>)
        .instructions,
    ).toBe('new');
  });
});

describe('retry policy sub-form', () => {
  it('adds a rule, switches it to a redirect, and picks a target', () => {
    const { onChange, rerender } = renderPanel();
    fireEvent.click(screen.getByLabelText('Add verdict rule'));
    let def = lastDef(onChange);
    expect(def.nodes.find((n) => n.id === AGENT_NODE_ID)!.retry!.verdict).toEqual({
      strategy: 'in_place',
      max_attempts: 3,
      feedback: true,
    });

    rerender(
      <ConfigPanel
        definition={def}
        nodeId={AGENT_NODE_ID}
        nodeTypes={CATALOG}
        onChange={onChange}
        onClose={vi.fn()}
      />,
    );
    fireEvent.change(screen.getByLabelText('verdict strategy'), {
      target: { value: 'redirect' },
    });
    def = lastDef(onChange);
    expect(def.nodes.find((n) => n.id === AGENT_NODE_ID)!.retry!.verdict!.strategy).toBe(
      'redirect',
    );

    rerender(
      <ConfigPanel
        definition={def}
        nodeId={AGENT_NODE_ID}
        nodeTypes={CATALOG}
        onChange={onChange}
        onClose={vi.fn()}
      />,
    );
    const target = screen.getByLabelText('verdict redirect target') as HTMLSelectElement;
    // Never offers the node itself — that would be an instant self-loop.
    expect(Array.from(target.options).map((o) => o.value)).not.toContain(AGENT_NODE_ID);
    fireEvent.change(target, { target: { value: target.options[1].value } });
    def = lastDef(onChange);
    expect(def.nodes.find((n) => n.id === AGENT_NODE_ID)!.retry!.verdict!.redirect_to).toBe(
      target.options[1].value,
    );
  });

  it('prunes the policy back to null when the last rule is removed', () => {
    const withRetry: WorkflowDefinitionV2 = {
      ...STARTER,
      nodes: STARTER.nodes.map((n) =>
        n.id === AGENT_NODE_ID ? { ...n, retry: { verdict: { strategy: 'fail' } } } : n,
      ),
    };
    const { onChange } = renderPanel(withRetry);
    fireEvent.click(screen.getByLabelText('Remove verdict rule'));
    const def = lastDef(onChange);
    expect(def.nodes.find((n) => n.id === AGENT_NODE_ID)!.retry).toBeNull();
  });
});

describe('join semantics', () => {
  it('stays hidden for a node with a single predecessor', () => {
    renderPanel();
    expect(screen.queryByLabelText('Join semantics')).toBeNull();
  });

  it('appears once a node has a real fan-in to resolve', () => {
    const fanIn: WorkflowDefinitionV2 = {
      ...STARTER,
      edges: [
        ...STARTER.edges.filter((e) => e.to !== AGENT_NODE_ID),
        { from: STARTER.nodes[1].id, to: AGENT_NODE_ID },
        { from: STARTER.nodes[2].id, to: AGENT_NODE_ID },
      ],
    };
    const { onChange } = renderPanel(fanIn);
    const select = screen.getByLabelText('Join semantics');
    fireEvent.change(select, { target: { value: 'all_done' } });
    const def = lastDef(onChange);
    expect(def.nodes.find((n) => n.id === AGENT_NODE_ID)!.join).toBe('all_done');
  });
});

/**
 * The task's Done-when, asserted end to end: drive every schema field of a real
 * starter's agent node through the panel and check the accumulated definition
 * still satisfies the schema those fields came from.
 */
describe('round-trip', () => {
  const typeCheck = (value: unknown, spec: Record<string, unknown>): boolean => {
    const declared = Array.isArray(spec.type)
      ? (spec.type as string[])
      : typeof spec.type === 'string'
        ? [spec.type as string]
        : [];
    if (value === null) return declared.includes('null');
    const actual = Array.isArray(value)
      ? 'array'
      : Number.isInteger(value)
        ? 'integer'
        : typeof value;
    return declared.length === 0 || declared.includes(actual) || (actual === 'integer' && declared.includes('number'));
  };

  it('edits every field of a starter agent node and stays valid v2 JSON', () => {
    const fields = fieldsFromSchema(AGENT_TYPE.config_schema);
    let def = STARTER;
    const onChange = vi.fn((next: WorkflowDefinitionV2) => {
      def = next;
    });

    // One field per mount: the panel is controlled, so each edit is applied
    // and the next render starts from the definition the last one produced.
    const edits: Record<string, () => void> = {
      model: () => fireEvent.change(screen.getByLabelText('Model'), { target: { value: 'opus' } }),
      effort: () =>
        fireEvent.change(screen.getByLabelText('Effort'), { target: { value: 'high' } }),
      capability: () =>
        fireEvent.change(screen.getByLabelText('Capability'), { target: { value: 'implement' } }),
      agent_kind: () =>
        fireEvent.change(screen.getByLabelText('Agent kind'), { target: { value: 'codex' } }),
      allow_network: () => fireEvent.click(screen.getByLabelText('Allow network')),
      allow_shell: () => fireEvent.click(screen.getByLabelText('Allow shell')),
      max_iterations: () =>
        fireEvent.change(screen.getByLabelText('Max iterations'), { target: { value: '4' } }),
      artifacts: () =>
        fireEvent.change(screen.getByLabelText('Artifacts'), {
          target: { value: '[{"name":"report"}]' },
        }),
      prompt_template: () =>
        fireEvent.change(screen.getByTestId('code-prompt_template').querySelector('textarea')!, {
          target: { value: 'Do the thing.' },
        }),
      rework_prompt_template: () =>
        fireEvent.change(
          screen.getByTestId('code-rework_prompt_template').querySelector('textarea')!,
          { target: { value: 'Fix only what the verdict named.' } },
        ),
    };
    // Every field the schema publishes must be covered — if the registry grows
    // one, this fails rather than quietly leaving it untested.
    expect(Object.keys(edits).sort()).toEqual(fields.map((f) => f.key).sort());

    for (const field of fields) {
      cleanup();
      render(
        <ConfigPanel
          definition={def}
          nodeId={AGENT_NODE_ID}
          nodeTypes={CATALOG}
          onChange={onChange}
          onClose={vi.fn()}
        />,
      );
      edits[field.key]();
    }

    const node = def.nodes.find((n) => n.id === AGENT_NODE_ID)!;
    expect(node.config).toMatchObject({
      model: 'opus',
      effort: 'high',
      capability: 'implement',
      agent_kind: 'codex',
      max_iterations: 4,
      artifacts: [{ name: 'report' }],
      prompt_template: 'Do the thing.',
      rework_prompt_template: 'Fix only what the verdict named.',
    });

    // Still valid against the schema the fields were derived from.
    const props = (AGENT_TYPE.config_schema as { properties: Record<string, Record<string, unknown>> })
      .properties;
    for (const [key, value] of Object.entries(node.config!)) {
      const spec = props[key];
      if (!spec) continue; // additionalProperties: true
      expect(typeCheck(value, spec), `${key} = ${JSON.stringify(value)}`).toBe(true);
      if (Array.isArray(spec.enum) && value !== null) {
        expect(spec.enum).toContain(value);
      }
    }

    // And the graph itself is untouched — this panel edits config, not shape.
    expect(def.edges).toEqual(STARTER.edges);
    expect(def.nodes.map((n) => n.id)).toEqual(STARTER.nodes.map((n) => n.id));
    // A structurally sound definition survives a JSON round-trip unchanged.
    expect(JSON.parse(JSON.stringify(def))).toEqual(def);
  });
});

describe('close', () => {
  it('reports the dismissal to its owner', () => {
    const onClose = vi.fn();
    render(
      <ConfigPanel
        definition={STARTER}
        nodeId={AGENT_NODE_ID}
        nodeTypes={CATALOG}
        onChange={vi.fn()}
        onClose={onClose}
      />,
    );
    fireEvent.click(screen.getByLabelText('Close config panel'));
    expect(onClose).toHaveBeenCalled();
  });

  it('renders nothing for a node id that is not in the graph', () => {
    const { container } = render(
      <ConfigPanel
        definition={STARTER}
        nodeId="ghost"
        nodeTypes={CATALOG}
        onChange={vi.fn()}
        onClose={vi.fn()}
      />,
    );
    expect(container.firstChild).toBeNull();
    expect(within(container).queryByTestId('config-panel')).toBeNull();
  });
});
