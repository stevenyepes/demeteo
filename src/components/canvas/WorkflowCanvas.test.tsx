/**
 * Fixture-driven coverage for the canvas foundation (task P2.1). The fixtures
 * are emitted from the live Rust v1→v2 migration (see the
 * `canvas_fixtures_are_current` regen test in demeteo-core), so this proves the
 * canvas renders exactly what the engine migrates for all seven bundled
 * starters — the P2.1 "Done when".
 *
 * Two levels: the pure `toFlowGraph` transform is asserted exhaustively over
 * every starter (robust, no DOM), and `WorkflowCanvas` is smoke-mounted once
 * per starter under jsdom (React Flow needs a handful of browser APIs jsdom
 * lacks — stubbed below) to prove it renders node titles with no console
 * errors.
 */
import type { ComponentProps } from 'react';
import type { ElkNode } from 'elkjs/lib/elk-api.js';
import { act, render, screen, cleanup, within } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { WorkflowCanvas } from './WorkflowCanvas';
import { toFlowGraph } from './flowGraph';
import {
  FIT_PADDING,
  MAX_ZOOM,
  MIN_ZOOM,
  MINIMAP_NODE_THRESHOLD,
} from './layoutDirection';
import { nodeTypeMeta, type NodeRunStatus, type WorkflowDefinitionV2 } from './types';

/** Props every `ReactFlow` render was handed, newest last. Recorded by the
 *  spy below so the zoom bounds can be asserted against the exported
 *  constants — the canvas must not re-declare them (spec AC-3). */
const { reactFlowProps } = vi.hoisted(() => ({
  reactFlowProps: [] as Record<string, unknown>[],
}));

// A *recording* spy, not a stand-in: it delegates to the real `ReactFlow`, so
// the seven fixture mounts below still exercise the actual canvas. A double
// that answered with a bland render instead would let every one of them pass
// while proving nothing (AGENTS §7).
vi.mock('@xyflow/react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@xyflow/react')>();
  const Recording = (props: ComponentProps<typeof actual.ReactFlow>) => {
    reactFlowProps.push(props as unknown as Record<string, unknown>);
    return <actual.ReactFlow {...props} />;
  };
  return { ...actual, ReactFlow: Recording };
});

/** How many times the canvas has re-planned its layout. `plan` is a `useMemo`
 *  whose only volatile dependency is the measured container, and the
 *  auto-layout effect takes that memo's identity as a dependency in turn — so
 *  this count is both the re-plan count and an upper bound on the `fitView` /
 *  elk re-entries a resize can trigger. */
const { planLayoutCalls } = vi.hoisted(() => ({ planLayoutCalls: { count: 0 } }));

vi.mock('./layoutDirection', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./layoutDirection')>();
  return {
    ...actual,
    planLayout: (...args: Parameters<typeof actual.planLayout>) => {
      planLayoutCalls.count += 1;
      return actual.planLayout(...args);
    },
  };
});

/** Every graph the canvas handed elk, oldest first. The worker is replaced
 *  rather than spawned: jsdom has no `Worker`, and this double answers with
 *  a position for exactly the children it was given. */
const { elkGraphs } = vi.hoisted(() => ({ elkGraphs: [] as ElkNode[] }));

vi.mock('elkjs/lib/elk-api.js', () => ({
  default: class {
    async layout(graph: ElkNode): Promise<ElkNode> {
      elkGraphs.push(graph);
      return {
        ...graph,
        children: (graph.children ?? []).map((c, i) => ({ ...c, x: i * 400, y: 0 })),
      };
    }
    terminateWorker() {}
  },
}));

import bugfix from './__fixtures__/bugfix-pipeline.v2.json';
import cifix from './__fixtures__/ci-fix.v2.json';
import codeReview from './__fixtures__/code-review.v2.json';
import docsUpdate from './__fixtures__/docs-update.v2.json';
import experiment from './__fixtures__/experiment.v2.json';
import refactor from './__fixtures__/refactor.v2.json';
import simpleTask from './__fixtures__/simple-task.v2.json';
import standard from './__fixtures__/standard-feature-pipeline.v2.json';

const STARTERS: [string, WorkflowDefinitionV2][] = [
  ['bugfix-pipeline', bugfix as unknown as WorkflowDefinitionV2],
  ['ci-fix', cifix as unknown as WorkflowDefinitionV2],
  ['code-review', codeReview as unknown as WorkflowDefinitionV2],
  ['docs-update', docsUpdate as unknown as WorkflowDefinitionV2],
  ['experiment', experiment as unknown as WorkflowDefinitionV2],
  ['refactor', refactor as unknown as WorkflowDefinitionV2],
  ['simple-task', simpleTask as unknown as WorkflowDefinitionV2],
  ['standard-feature-pipeline', standard as unknown as WorkflowDefinitionV2],
];

describe('toFlowGraph', () => {
  it.each(STARTERS)('maps every node and edge of %s', (_name, def) => {
    const { nodes, edges } = toFlowGraph(def);

    expect(nodes).toHaveLength(def.nodes.length);
    expect(edges).toHaveLength(def.edges.length);

    for (const node of nodes) {
      const source = def.nodes.find((n) => n.id === node.id)!;
      expect(source).toBeDefined();
      expect(node.type).toBe('workflow');
      expect(node.data.title).toBe(source.title);
      expect(node.data.nodeType).toBe(source.type);
      // Every node lands somewhere and resolves to display metadata.
      expect(Number.isFinite(node.position.x)).toBe(true);
      expect(Number.isFinite(node.position.y)).toBe(true);
      expect(nodeTypeMeta(node.data.nodeType).label).toBeTruthy();
    }

    // Edge ids are unique (React Flow requires it) and endpoints are real.
    const ids = new Set(edges.map((e) => e.id));
    expect(ids.size).toBe(edges.length);
    const nodeIds = new Set(nodes.map((n) => n.id));
    for (const e of edges) {
      expect(nodeIds.has(e.source)).toBe(true);
      expect(nodeIds.has(e.target)).toBe(true);
    }
  });

  it('threads the run-mode overlay (P2.2) onto matching nodes only', () => {
    const def: WorkflowDefinitionV2 = {
      schema_version: 2,
      id: 'wf-x',
      name: 'X',
      nodes: [
        { id: 'a', type: 'agent', title: 'A' },
        { id: 'b', type: 'gate', title: 'B' },
      ],
      edges: [{ from: 'a', to: 'b' }],
    };
    const { nodes } = toFlowGraph(def, {
      statusByNode: {
        a: { status: 'completed', costUsd: 0.42, wallClockSecs: 12, stepExecutionId: 'se-a' },
      },
    });
    const a = nodes.find((n) => n.id === 'a')!;
    const b = nodes.find((n) => n.id === 'b')!;
    expect(a.data.run?.status).toBe('completed');
    expect(a.data.run?.costUsd).toBe(0.42);
    // A node with no overlay entry stays in design-mode (no run state).
    expect(b.data.run).toBeUndefined();
  });

  it('labels conditional (`when`) edges and leaves chain edges bare', () => {
    const def: WorkflowDefinitionV2 = {
      schema_version: 2,
      id: 'wf-x',
      name: 'X',
      nodes: [
        { id: 'a', type: 'agent', title: 'A' },
        { id: 'b', type: 'gate', title: 'B' },
      ],
      edges: [{ from: 'a', to: 'b', when: "${{ nodes.a.outputs.verdict != 'FAIL' }}" }],
    };
    const { edges } = toFlowGraph(def);
    expect(edges[0].label).toBe('when');
    expect(edges[0].data?.when).toContain('verdict');
  });
});

// Shared by every mounting suite in this file (the fixture mounts and the zoom
// / minimap cases below), which is why it sits at file scope rather than inside
// one `describe`.
/** Every observer the mounted tree created. The setup file's shared stub fires
 *  with an empty entry list, which the canvas's own observer cannot read — it
 *  takes its size from `entry.contentRect` — so this file keeps its own and
 *  hands the callback entries with a box in them. */
const observers: RO[] = [];

class RO implements ResizeObserver {
  targets: Element[] = [];

  constructor(readonly callback: ResizeObserverCallback) {
    observers.push(this);
  }

  observe(el: Element) {
    this.targets.push(el);
  }
  unobserve() {}
  disconnect() {}

  /** Drive one resize tick. jsdom lays nothing out, so every box under test is
   *  the one stated here. */
  tick(width: number, height: number): void {
    const rect = { x: 0, y: 0, top: 0, left: 0, right: width, bottom: height, width, height };
    this.callback([{ contentRect: rect } as unknown as ResizeObserverEntry], this);
  }
}

/** The observer watching the canvas wrapper, as opposed to the ones React Flow
 *  installs on the viewport and the panes. */
function canvasObserver(): RO {
  const wrapper = screen.getByTestId('workflow-canvas');
  const found = observers.find((o) => o.targets.includes(wrapper));
  if (!found) throw new Error('the canvas installed no ResizeObserver');
  return found;
}

beforeAll(() => {
  // React Flow reaches for browser APIs jsdom doesn't implement. Stub the
  // minimal set so a mount renders node DOM without env warnings.
  vi.stubGlobal('ResizeObserver', RO);

  class DOMMatrixStub {
    m22 = 1;
    constructor() {}
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

  // Give every element a non-zero box so React Flow doesn't warn about a
  // zero-dimension container.
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

describe('WorkflowCanvas render', () => {
  let consoleError: ReturnType<typeof vi.spyOn>;

  afterEach(() => {
    cleanup();
    consoleError?.mockRestore();
  });

  it.each(STARTERS)('renders %s node titles without console errors', (_name, def) => {
    consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});

    render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas definition={def} />
      </div>,
    );

    // Every node's title is in the DOM.
    for (const node of def.nodes) {
      expect(screen.getAllByText(node.title).length).toBeGreaterThan(0);
    }
    expect(consoleError).not.toHaveBeenCalled();
  });

  it('shows the observed agent and effective effort on only the matching run node', () => {
    const longAgentKind = 'custom-agent-kind-with-a-name-longer-than-the-node-badge';
    const def: WorkflowDefinitionV2 = {
      schema_version: 2,
      id: 'wf-assignments',
      name: 'Assignments',
      nodes: [
        { id: 'implement', type: 'agent', title: 'Implement' },
        { id: 'approval', type: 'gate', title: 'Approval' },
        { id: 'pending', type: 'agent', title: 'Pending work' },
      ],
      edges: [
        { from: 'implement', to: 'approval' },
        { from: 'approval', to: 'pending' },
      ],
    };

    render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas
          definition={def}
          statusByNode={{
            implement: {
              status: 'completed',
              stepExecutionId: 'se-implement',
              agentKind: longAgentKind,
              effort: 'xhigh',
            },
            approval: { status: 'awaiting_gate', stepExecutionId: 'se-approval' },
            pending: { status: 'pending', stepExecutionId: 'se-pending' },
          }}
        />
      </div>,
    );

    // Truncation is CSS, so the full value has to survive somewhere a pointer
    // and a screen reader can each reach it: the badge title and the one
    // composed label the pair is announced under.
    const assignment = screen.getByLabelText(
      `Actual assignment for Implement: Agent: ${longAgentKind}; Effective effort: Extra high`,
    );
    expect(within(assignment).getByTitle(`Agent: ${longAgentKind}`)).toHaveTextContent(
      longAgentKind,
    );
    expect(within(assignment).getByTitle('Effective effort: Extra high')).toHaveTextContent(
      'Extra high',
    );
    expect(screen.queryByLabelText(/Actual assignment for Approval/)).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/Actual assignment for Pending work/)).not.toBeInTheDocument();
    expect(screen.queryByText('Max')).not.toBeInTheDocument();
  });

  it('shows the neutral wording for an observed null effective effort', () => {
    const def: WorkflowDefinitionV2 = {
      schema_version: 2,
      id: 'wf-null-effort',
      name: 'Null effort',
      nodes: [{ id: 'implement', type: 'agent', title: 'Implement' }],
      edges: [],
    };

    render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas
          definition={def}
          statusByNode={{
            implement: {
              status: 'completed',
              stepExecutionId: 'se-implement',
              agentKind: 'hermes',
              effort: null,
            },
          }}
        />
      </div>,
    );

    const assignment = screen.getByLabelText(
      'Actual assignment for Implement: Agent: hermes; Effective effort: No injected effort',
    );
    expect(within(assignment).getByTitle('Agent: hermes')).toBeInTheDocument();
    expect(within(assignment).queryByText('High')).not.toBeInTheDocument();
  });

  it('shows the full pinned model id on the run node it was spawned with', () => {
    const longModel = 'anthropic/claude-sonnet-4-5-20250929-with-a-provider-routed-suffix';
    expect(longModel.length).toBeGreaterThan(60);
    const def: WorkflowDefinitionV2 = {
      schema_version: 2,
      id: 'wf-model',
      name: 'Model',
      nodes: [{ id: 'implement', type: 'agent', title: 'Implement' }],
      edges: [],
    };

    render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas
          definition={def}
          statusByNode={{
            implement: {
              status: 'completed',
              stepExecutionId: 'se-implement',
              agentKind: 'claude-code',
              model: longModel,
              effort: 'high',
            },
          }}
        />
      </div>,
    );

    const assignment = screen.getByLabelText(
      `Actual assignment for Implement: Agent: claude-code; Model: ${longModel}; Effective effort: High`,
    );
    expect(within(assignment).getByTitle(`Model: ${longModel}`)).toHaveTextContent(longModel);
  });

  it('keeps configured design-mode essence separate from actual assignment metadata', () => {
    const def: WorkflowDefinitionV2 = {
      schema_version: 2,
      id: 'wf-design-assignment',
      name: 'Design assignment',
      nodes: [
        {
          id: 'implement',
          type: 'agent',
          title: 'Implement',
          config: { agent_kind: 'configured-agent', effort: 'max' },
        },
      ],
      edges: [],
    };

    render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas definition={def} mode="design" />
      </div>,
    );

    expect(screen.getByTestId('node-essence')).toHaveTextContent('configured-agent');
    expect(screen.getByTestId('node-essence')).toHaveTextContent('max');
    expect(screen.queryByLabelText(/Actual assignment for Implement/)).not.toBeInTheDocument();
  });
});

/**
 * The zoom bounds and the fit-view margin are `layoutDirection`'s, not the
 * canvas's: `planLayout` estimates the fit scale against exactly the clamp
 * React Flow will apply, so a literal re-hardcoded here would silently put the
 * estimate and the real viewport back out of step (spec Constraint 10).
 */
describe('WorkflowCanvas zoom bounds', () => {
  beforeEach(() => {
    reactFlowProps.length = 0;
  });

  afterEach(cleanup);

  it('hands ReactFlow the exported zoom constants, not its own', () => {
    render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas definition={simpleTask as unknown as WorkflowDefinitionV2} />
      </div>,
    );

    const props = reactFlowProps[reactFlowProps.length - 1];
    expect(props.maxZoom).toBe(MAX_ZOOM);
    expect(props.minZoom).toBe(MIN_ZOOM);
  });

  it('fits the view with the same clamp and margin the estimate assumes', () => {
    render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas definition={simpleTask as unknown as WorkflowDefinitionV2} />
      </div>,
    );

    const options = reactFlowProps[reactFlowProps.length - 1].fitViewOptions;
    expect(options).toEqual(
      expect.objectContaining({ maxZoom: MAX_ZOOM, padding: 1 - FIT_PADDING }),
    );
  });
});

/**
 * The minimap is now `needsMiniMap`'s call rather than a threshold spelled in
 * the component. Under jsdom the `ResizeObserver` stub never reports a box, so
 * the plan carries no fit scale and the predicate falls back to node count —
 * which is the branch these two starters straddle. The scale branch is covered
 * where it is decidable, in `layoutDirection.test.ts`.
 */
describe('WorkflowCanvas minimap', () => {
  afterEach(cleanup);

  it('shows the minimap for a graph at the node-count threshold', () => {
    const def = standard as unknown as WorkflowDefinitionV2;
    expect(def.nodes.length).toBeGreaterThanOrEqual(MINIMAP_NODE_THRESHOLD);

    const { container } = render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas definition={def} />
      </div>,
    );
    expect(container.querySelector('.react-flow__minimap')).not.toBeNull();
  });

  it('hides it for a graph below the threshold that fits legibly', () => {
    const def = simpleTask as unknown as WorkflowDefinitionV2;
    expect(def.nodes.length).toBeLessThan(MINIMAP_NODE_THRESHOLD);

    const { container } = render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas definition={def} />
      </div>,
    );
    expect(container.querySelector('.react-flow__minimap')).toBeNull();
  });
});

/**
 * Dragging the inspector divider resizes this canvas's grid track directly, so
 * `SplitPane` holding the drag out of React (UI_REDESIGN_PLAN §4.1) spares the
 * run column's observer but not the canvas's own. Undamped, a 400px drag
 * commits ~50 sizes past the 8px rounding, and each one re-plans the layout and
 * restarts the `fitView` animation the last one began.
 */
describe('WorkflowCanvas resize damping', () => {
  beforeEach(() => {
    observers.length = 0;
    planLayoutCalls.count = 0;
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    cleanup();
  });

  it('re-plans once for a burst of resize ticks, not once per tick', () => {
    render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas definition={simpleTask as unknown as WorkflowDefinitionV2} />
      </div>,
    );
    const observer = canvasObserver();

    // Settle the mount: the canvas has no layout at all until it has been
    // measured once, so that first tick is expected to land immediately.
    act(() => observer.tick(800, 600));
    act(() => void vi.advanceTimersByTime(1000));
    planLayoutCalls.count = 0;

    // The drag: 50 distinct boxes, every one of them past the rounding. Each
    // tick gets its own `act` because each pointer move is its own task in the
    // browser — batching the burst into one would let React coalesce the very
    // renders this is measuring.
    for (let width = 792; width >= 400; width -= 8) {
      act(() => observer.tick(width, 600));
    }
    expect(planLayoutCalls.count).toBe(0);

    act(() => void vi.advanceTimersByTime(1000));
    expect(planLayoutCalls.count).toBe(1);
  });

  it('still re-plans for a resize that ends where the last one did not', () => {
    render(
      <div style={{ width: 800, height: 600 }}>
        <WorkflowCanvas definition={simpleTask as unknown as WorkflowDefinitionV2} />
      </div>,
    );
    const observer = canvasObserver();

    act(() => observer.tick(800, 600));
    act(() => void vi.advanceTimersByTime(1000));
    planLayoutCalls.count = 0;

    act(() => observer.tick(400, 600));
    act(() => void vi.advanceTimersByTime(1000));
    act(() => observer.tick(1600, 600));
    act(() => void vi.advanceTimersByTime(1000));

    expect(planLayoutCalls.count).toBe(2);
  });
});

/**
 * Elk places each card at the size it had when it ran. A run-mode card grows
 * after that — the assignment chips arrive with `agent_spawned` — and growing
 * in place past the 64px layer gap overlaps the next card, so a measured size
 * change has to reach elk. A status tick must not, including across the
 * unmeasured render its re-seed causes (the `measured` guard in the canvas).
 */
describe('WorkflowCanvas re-layout on a measured size change', () => {
  const def = simpleTask as unknown as WorkflowDefinitionV2;
  /** What each card measures, by node id. jsdom lays nothing out, so this is
   *  the only source of `offsetWidth` / `offsetHeight` React Flow reads. */
  const sizes = new Map<string, { width: number; height: number }>();
  const offset = {
    width: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetWidth'),
    height: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight'),
  };

  const statuses = (implement: string): Record<string, NodeRunStatus> =>
    Object.fromEntries(
      def.nodes.map((n) => [n.id, { status: n.id === 's-implement' ? implement : 'pending' }]),
    );

  /** Deliver the node ResizeObserver's callback for every card still mounted,
   *  the way a browser does once a card is observed or resized. */
  const measureNodes = () => {
    for (const o of observers) {
      const cards = o.targets.filter((t) => t.isConnected && t.classList.contains('react-flow__node'));
      if (cards.length === 0) continue;
      o.callback(cards.map((target) => ({ target }) as unknown as ResizeObserverEntry), o);
    }
  };

  /** Let measurement, the elk promise and every re-seed they cause run out. */
  const settle = async () => {
    for (let i = 0; i < 6; i++) {
      act(measureNodes);
      await act(async () => {
        await Promise.resolve();
      });
    }
  };

  const widthFedFor = (graph: ElkNode | undefined, id: string) =>
    graph?.children?.find((c) => c.id === id)?.width;

  beforeEach(() => {
    observers.length = 0;
    elkGraphs.length = 0;
    sizes.clear();
    for (const n of def.nodes) sizes.set(n.id, { width: 200, height: 80 });
    const dimension = (axis: 'width' | 'height') =>
      function (this: HTMLElement) {
        const id = this.classList.contains('react-flow__node') ? this.getAttribute('data-id') : null;
        return id ? (sizes.get(id)?.[axis] ?? 0) : axis === 'width' ? 1600 : 600;
      };
    Object.defineProperty(HTMLElement.prototype, 'offsetWidth', {
      configurable: true,
      get: dimension('width'),
    });
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
      configurable: true,
      get: dimension('height'),
    });
  });

  afterEach(() => {
    cleanup();
    if (offset.width) Object.defineProperty(HTMLElement.prototype, 'offsetWidth', offset.width);
    if (offset.height) Object.defineProperty(HTMLElement.prototype, 'offsetHeight', offset.height);
  });

  async function mountLaidOut() {
    const view = render(
      <div style={{ width: 1600, height: 600 }}>
        <WorkflowCanvas definition={def} statusByNode={statuses('pending')} />
      </div>,
    );
    act(() => canvasObserver().tick(1600, 600));
    await settle();
    expect(elkGraphs).toHaveLength(1);
    return view;
  }

  it('lays out once, at the measured size, and not again once it settles', async () => {
    await mountLaidOut();
    expect(widthFedFor(elkGraphs[0], 's-implement')).toBe(200);
  });

  it('does not re-run elk for a status tick that resizes nothing', async () => {
    const { rerender } = await mountLaidOut();

    rerender(
      <div style={{ width: 1600, height: 600 }}>
        <WorkflowCanvas definition={def} statusByNode={statuses('running')} />
      </div>,
    );
    await settle();

    expect(elkGraphs).toHaveLength(1);
  });

  it('re-runs elk when a card widens in place, at the new width', async () => {
    await mountLaidOut();

    sizes.set('s-implement', { width: 280, height: 80 });
    await settle();

    expect(elkGraphs).toHaveLength(2);
    expect(widthFedFor(elkGraphs[1], 's-implement')).toBe(280);
  });
});
