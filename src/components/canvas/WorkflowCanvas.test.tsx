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
import { render, screen, cleanup } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { WorkflowCanvas } from './WorkflowCanvas';
import { toFlowGraph } from './flowGraph';
import { nodeTypeMeta, type WorkflowDefinitionV2 } from './types';

import bugfix from './__fixtures__/bugfix-pipeline.v2.json';
import cifix from './__fixtures__/ci-fix.v2.json';
import docsUpdate from './__fixtures__/docs-update.v2.json';
import experiment from './__fixtures__/experiment.v2.json';
import refactor from './__fixtures__/refactor.v2.json';
import simpleTask from './__fixtures__/simple-task.v2.json';
import standard from './__fixtures__/standard-feature-pipeline.v2.json';

const STARTERS: [string, WorkflowDefinitionV2][] = [
  ['bugfix-pipeline', bugfix as unknown as WorkflowDefinitionV2],
  ['ci-fix', cifix as unknown as WorkflowDefinitionV2],
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

describe('WorkflowCanvas render', () => {
  let consoleError: ReturnType<typeof vi.spyOn>;

  beforeAll(() => {
    // React Flow reaches for browser APIs jsdom doesn't implement. Stub the
    // minimal set so a mount renders node DOM without env warnings.
    class RO {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
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
});
