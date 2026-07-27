/**
 * Design-mode surfaces (task P3.1): the registry-driven palette, the Cmd+K /
 * connect-drop picker, and the canvas wiring that turns a pick into an edit.
 *
 * The load-bearing assertion is `renders a node type it has never heard of`:
 * the palette is built entirely from what `node_types_list` returns, which is
 * what makes P3.5's "the `command` node appears in the builder with zero
 * frontend edits" true by construction rather than by a follow-up commit.
 */
import { render, screen, cleanup, fireEvent } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { Palette, NodeTypePicker, NODE_TYPE_MIME } from './Palette';
import { WorkflowCanvas } from './WorkflowCanvas';
import catalogFixture from './__fixtures__/node_catalog.json';
import type { NodeTypeInfo } from './nodeCatalog';
import type { WorkflowDefinitionV2 } from './types';

const CATALOG: NodeTypeInfo[] = [
  {
    kind: 'agent',
    label: 'Agent',
    summary: 'One agent turn against the feature worktree.',
    config_schema: { type: 'object' },
    inputs: ['any'],
    outputs: ['text', 'file', 'task_list', 'verdict'],
    max_instances: null,
  },
  {
    kind: 'gate',
    label: 'Gate',
    summary: 'Pause for a human decision.',
    config_schema: { type: 'object' },
    inputs: ['any'],
    outputs: ['approval'],
    max_instances: null,
  },
  {
    kind: 'finalize',
    label: 'Finalize',
    summary: 'Squash and publish. Ends the run.',
    config_schema: { type: 'object' },
    inputs: ['any'],
    outputs: [],
    max_instances: 1,
  },
];

const DEF: WorkflowDefinitionV2 = {
  schema_version: 2,
  id: 'wf',
  name: 'Demo',
  nodes: [
    { id: 'a', type: 'agent', title: 'Research', position: { x: 0, y: 0 } },
    { id: 'z', type: 'finalize', title: 'Ship', position: { x: 0, y: 320 } },
  ],
  edges: [{ from: 'a', to: 'z' }],
};

const entries = (def: WorkflowDefinitionV2 = DEF) =>
  CATALOG.map((type) => ({
    type,
    disabledReason:
      type.max_instances != null &&
      def.nodes.filter((n) => n.type === type.kind).length >= type.max_instances
        ? `Only ${type.max_instances} ${type.label} node per workflow.`
        : undefined,
  }));

afterEach(cleanup);

describe('Palette', () => {
  it('renders one entry per catalog type, from the registry alone', () => {
    render(<Palette entries={entries()} onSelect={vi.fn()} />);
    expect(screen.getByText('Agent')).toBeTruthy();
    expect(screen.getByText('Gate')).toBeTruthy();
    expect(screen.getByText('Finalize')).toBeTruthy();
  });

  it('renders a node type it has never heard of', () => {
    // The P3.5 acceptance test in miniature: a kind with no frontend
    // metadata still gets a labelled, selectable palette entry (falling back
    // to the generic icon in `types.ts`).
    const future: NodeTypeInfo = {
      kind: 'command',
      label: 'Command',
      summary: 'Run a deterministic shell command.',
      config_schema: { type: 'object' },
      inputs: ['any'],
      outputs: ['text', 'file'],
      max_instances: null,
    };
    const onSelect = vi.fn();
    render(<Palette entries={[{ type: future }]} onSelect={onSelect} />);

    fireEvent.click(screen.getByText('Command'));
    expect(onSelect).toHaveBeenCalledWith(future);
  });

  it('offers the `command` type from the real registry, with no frontend edit', () => {
    // The P3.5 acceptance test for real: `node_catalog.json` is emitted from
    // the live Rust registry, so this fails if the type is ever registered
    // without introducing itself — and it passed the day the type landed,
    // with no change to `Palette.tsx` or `types.ts`.
    const command = (catalogFixture as unknown as NodeTypeInfo[]).find(
      (t) => t.kind === 'command',
    );
    expect(command).toBeTruthy();

    const onSelect = vi.fn();
    render(<Palette entries={[{ type: command! }]} onSelect={vi.fn().mockImplementation(onSelect)} />);
    expect(screen.getByText('Command')).toBeTruthy();
    // It produces something, so an edge may leave it (unlike `finalize`).
    expect(command!.outputs.length).toBeGreaterThan(0);
  });

  it('disables a type at its instance cap and says why', () => {
    const onSelect = vi.fn();
    render(<Palette entries={entries()} onSelect={onSelect} />);

    // `DEF` already holds the one allowed finalize.
    expect(screen.getByText('Only 1 Finalize node per workflow.')).toBeTruthy();
    fireEvent.click(screen.getByText('Finalize'));
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('filters by label, kind, or summary', () => {
    render(<Palette entries={entries()} onSelect={vi.fn()} />);
    fireEvent.change(screen.getByLabelText('Search node types'), {
      target: { value: 'human' },
    });
    expect(screen.getByText('Gate')).toBeTruthy();
    expect(screen.queryByText('Agent')).toBeNull();
  });

  it('carries the node kind on the drag payload', () => {
    render(<Palette entries={entries()} onSelect={vi.fn()} />);
    const setData = vi.fn();
    fireEvent.dragStart(screen.getByText('Agent').closest('button')!, {
      dataTransfer: { setData, effectAllowed: '' },
    });
    expect(setData).toHaveBeenCalledWith(NODE_TYPE_MIME, 'agent');
  });
});

describe('NodeTypePicker', () => {
  it('selects with arrow keys and Enter', () => {
    const onSelect = vi.fn();
    render(
      <NodeTypePicker
        title="Add a node"
        entries={entries()}
        onSelect={onSelect}
        onDismiss={vi.fn()}
      />,
    );
    const input = screen.getByLabelText('Search node types');
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onSelect).toHaveBeenCalledWith(CATALOG[1]); // Gate
  });

  it('refuses to commit a capped entry', () => {
    const onSelect = vi.fn();
    render(
      <NodeTypePicker
        title="Add a node"
        entries={entries()}
        onSelect={onSelect}
        onDismiss={vi.fn()}
      />,
    );
    const input = screen.getByLabelText('Search node types');
    fireEvent.change(input, { target: { value: 'finalize' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('dismisses on Escape', () => {
    const onDismiss = vi.fn();
    render(
      <NodeTypePicker
        title="Add a node"
        entries={entries()}
        onSelect={vi.fn()}
        onDismiss={onDismiss}
      />,
    );
    fireEvent.keyDown(screen.getByLabelText('Search node types'), { key: 'Escape' });
    expect(onDismiss).toHaveBeenCalled();
  });

  it('explains an empty compatible-type list', () => {
    render(
      <NodeTypePicker title="Connect to a new node" entries={[]} onSelect={vi.fn()} onDismiss={vi.fn()} />,
    );
    expect(screen.getByText('Nothing here can accept that connection.')).toBeTruthy();
  });
});

describe('WorkflowCanvas design mode', () => {
  beforeAll(() => {
    // React Flow reaches for browser APIs jsdom lacks — same stubs as the
    // P2.1 canvas test.
    class RO {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    vi.stubGlobal('ResizeObserver', RO);
    class DOMMatrixStub {
      m22 = 1;
    }
    vi.stubGlobal('DOMMatrixReadOnly', DOMMatrixStub);
    vi.stubGlobal('DOMMatrix', DOMMatrixStub);
    window.matchMedia =
      window.matchMedia ||
      vi.fn().mockImplementation((query: string) => ({
        matches: false,
        media: query,
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      }));
    Object.defineProperty(HTMLElement.prototype, 'getBoundingClientRect', {
      configurable: true,
      value: () => ({
        x: 0,
        y: 0,
        width: 800,
        height: 600,
        top: 0,
        left: 0,
        right: 800,
        bottom: 600,
        toJSON: () => {},
      }),
    });
  });

  const mount = (props: Partial<React.ComponentProps<typeof WorkflowCanvas>> = {}) => {
    const onDefinitionChange = vi.fn();
    render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas
          definition={DEF}
          mode="design"
          nodeTypes={CATALOG}
          onDefinitionChange={onDefinitionChange}
          {...props}
        />
      </div>,
    );
    return { onDefinitionChange };
  };

  it('shows the palette in design mode only', () => {
    mount();
    expect(screen.getByTestId('node-palette')).toBeTruthy();
    cleanup();

    render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas definition={DEF} />
      </div>,
    );
    expect(screen.queryByTestId('node-palette')).toBeNull();
  });

  it('adds a node when a palette entry is clicked', () => {
    const { onDefinitionChange } = mount();
    fireEvent.click(screen.getByText('Gate'));

    expect(onDefinitionChange).toHaveBeenCalledTimes(1);
    const next: WorkflowDefinitionV2 = onDefinitionChange.mock.calls[0][0];
    expect(next.nodes.map((n) => n.id)).toEqual(['a', 'z', 'gate']);
    // Lands below the lowest existing node rather than on top of the graph.
    expect(next.nodes[2].position).toEqual({ x: 0, y: 480 });
    // Adding a node wires no edge on its own.
    expect(next.edges).toEqual(DEF.edges);
  });

  it('opens the search picker on Cmd+K and adds the picked type', () => {
    const { onDefinitionChange } = mount();
    fireEvent.keyDown(screen.getByTestId('workflow-canvas'), { key: 'k', metaKey: true });

    const picker = screen.getByTestId('node-type-picker');
    expect(picker).toBeTruthy();
    fireEvent.click(screen.getAllByText('Gate')[1]); // [0] is the palette rail

    const next: WorkflowDefinitionV2 = onDefinitionChange.mock.calls[0][0];
    expect(next.nodes.some((n) => n.type === 'gate')).toBe(true);
    expect(screen.queryByTestId('node-type-picker')).toBeNull();
  });

  it('ignores Cmd+K in run mode', () => {
    render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas definition={DEF} />
      </div>,
    );
    fireEvent.keyDown(screen.getByTestId('workflow-canvas'), { key: 'k', metaKey: true });
    expect(screen.queryByTestId('node-type-picker')).toBeNull();
  });

  it('adds a node from a palette drop at the drop point', () => {
    const { onDefinitionChange } = mount();
    fireEvent.drop(screen.getByTestId('workflow-canvas'), {
      clientX: 120,
      clientY: 240,
      dataTransfer: { getData: (t: string) => (t === NODE_TYPE_MIME ? 'gate' : ''), types: [NODE_TYPE_MIME] },
    });
    expect(onDefinitionChange).toHaveBeenCalledTimes(1);
    const next: WorkflowDefinitionV2 = onDefinitionChange.mock.calls[0][0];
    expect(next.nodes[2].type).toBe('gate');
  });

  it('refuses a drop that would exceed an instance cap, and explains', () => {
    const onConnectRejected = vi.fn();
    const { onDefinitionChange } = mount({ onConnectRejected });
    fireEvent.drop(screen.getByTestId('workflow-canvas'), {
      clientX: 120,
      clientY: 240,
      dataTransfer: {
        getData: (t: string) => (t === NODE_TYPE_MIME ? 'finalize' : ''),
        types: [NODE_TYPE_MIME],
      },
    });
    expect(onDefinitionChange).not.toHaveBeenCalled();
    expect(onConnectRejected).toHaveBeenCalledWith('Only 1 Finalize node per workflow.');
  });

  it('selects the node it just added so the config panel can open on it', () => {
    const onNodeActivate = vi.fn();
    mount({ onNodeActivate });
    fireEvent.click(screen.getByText('Gate'));
    expect(onNodeActivate).toHaveBeenCalledWith('gate');
  });
});
