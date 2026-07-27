/**
 * `WorkflowBuilderScreen` (task P3.6) — the route that finally connects the
 * builder to storage, and the template picker that starts a new one.
 *
 * The claims under test are P3.6's own: opening a workflow loads the *graph*
 * (not a step list) and saving sends that graph to `workflow_save` intact —
 * which is what the V34 `definition_json` column exists to make possible.
 * Everything the builder itself does (lint gate, dirty guard, undo) is
 * `WorkflowBuilder.test.tsx`'s job and is not re-asserted here.
 */
import { render, screen, cleanup, fireEvent, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke, type InvokeArgs } from '@tauri-apps/api/core';

import { NavigationProvider } from '../../context/NavigationContext';
import { WorkflowBuilderScreen } from './WorkflowBuilderScreen';
import catalogFixture from './__fixtures__/node_catalog.json';
import { resetNodeTypeCache, type NodeTypeInfo } from './nodeCatalog';
import { WORKFLOW_TEMPLATES, templateById } from './templates';
import type { WorkflowDefinitionV2 } from './types';

const CATALOG = catalogFixture as unknown as NodeTypeInfo[];

/** A graph carrying the things v1 storage cannot hold — the whole point of the
 *  save path this screen owns. */
const GRAPH: WorkflowDefinitionV2 = {
  schema_version: 2,
  id: 'wf-1',
  name: 'Bugfix',
  nodes: [
    {
      id: 'plan',
      type: 'agent',
      title: 'Plan',
      config: { prompt_template: 'plan it' },
      position: { x: 40, y: 0 },
    },
    {
      id: 'scan',
      type: 'agent',
      title: 'Security scan',
      config: { prompt_template: 'scan it' },
      position: { x: 240, y: 160 },
    },
    { id: 'ship', type: 'finalize', title: 'Ship', position: { x: 40, y: 320 } },
  ],
  edges: [
    { from: 'plan', to: 'scan' },
    { from: 'scan', to: 'ship' },
  ],
};

const ROW = {
  id: 'wf-1',
  name: 'Bugfix',
  description: 'fix the bug',
  is_starter: false,
  version: 3,
  version_id: 'wf-1-v3',
};

let saved: Array<Record<string, unknown>> = [];

function mockBackend() {
  vi.mocked(invoke).mockImplementation((cmd: string, args?: InvokeArgs) => {
    const a = (args ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case 'workflow_get':
        return Promise.resolve(ROW);
      case 'workflow_version_graph':
        return Promise.resolve(GRAPH);
      case 'workflow_list':
        return Promise.resolve([
          { id: 'wf-1', name: 'Bugfix', description: 'fix the bug', is_starter: false },
          { id: 'wf-s', name: 'Standard', description: 'the starter', is_starter: true },
        ]);
      case 'workflow_save':
        saved.push(a);
        return Promise.resolve({ ...ROW, id: 'wf-new-1', version: 1, version_id: 'wf-new-1-v1' });
      case 'node_types_list':
        return Promise.resolve(CATALOG);
      case 'workflow_lint':
        return Promise.resolve([]);
      default:
        return Promise.resolve(undefined);
    }
  });
}

beforeAll(() => {
  // React Flow needs these under jsdom (the P2.1 stubs).
  class RO {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = RO;
  (globalThis as { DOMMatrixReadOnly?: unknown }).DOMMatrixReadOnly = class {
    m22 = 1;
    constructor(_t?: string) {}
  };
  Object.defineProperty(HTMLElement.prototype, 'getBoundingClientRect', {
    configurable: true,
    value: () => ({ width: 900, height: 700, top: 0, left: 0, right: 900, bottom: 700, x: 0, y: 0 }),
  });
  if (!window.matchMedia) {
    Object.defineProperty(window, 'matchMedia', {
      writable: true,
      value: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
    });
  }
});

beforeEach(() => {
  saved = [];
  resetNodeTypeCache();
  mockBackend();
  localStorage.clear();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('opening an existing workflow', () => {
  it('loads the pinned version graph and hands it to the builder', async () => {
    render(
      <NavigationProvider>
        <WorkflowBuilderScreen workflowId="wf-1" onBack={() => {}} />
      </NavigationProvider>,
    );

    await waitFor(() => expect(screen.getByTestId('workflow-builder')).toBeTruthy());
    expect((screen.getByLabelText('Workflow name') as HTMLInputElement).value).toBe('Bugfix');
    // The *graph* was requested, keyed to the workflow's latest version — not
    // a step list the screen would have to migrate itself.
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('workflow_version_graph', {
      workflowId: 'wf-1',
      versionId: 'wf-1-v3',
    });
  });

  it('saves the graph verbatim, positions included', async () => {
    render(
      <NavigationProvider>
        <WorkflowBuilderScreen workflowId="wf-1" onBack={() => {}} />
      </NavigationProvider>,
    );
    await waitFor(() => expect(screen.getByTestId('workflow-builder')).toBeTruthy());

    fireEvent.click(screen.getByTitle(/Save a new version/));
    await waitFor(() => expect(saved.length).toBe(1));

    const definition = saved[0].definition as WorkflowDefinitionV2;
    expect(saved[0].workflowId).toBe('wf-1');
    expect(definition.nodes.map((n) => n.id)).toEqual(['plan', 'scan', 'ship']);
    expect(definition.nodes[1].position).toEqual({ x: 240, y: 160 });
    expect(definition.edges).toHaveLength(2);
  });

  it('surfaces a load failure instead of an empty canvas', async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) =>
      cmd === 'workflow_get' ? Promise.reject('boom') : Promise.resolve(undefined),
    );
    render(
      <NavigationProvider>
        <WorkflowBuilderScreen workflowId="wf-1" onBack={() => {}} />
      </NavigationProvider>,
    );
    await waitFor(() => expect(screen.getByText(/boom/)).toBeTruthy());
    expect(screen.queryByTestId('workflow-builder')).toBeNull();
  });
});

describe('starting a new workflow', () => {
  it('offers the template picker rather than a blank builder', async () => {
    render(
      <NavigationProvider>
        <WorkflowBuilderScreen workflowId={null} onBack={() => {}} />
      </NavigationProvider>,
    );
    await waitFor(() => expect(screen.getByTestId('template-picker')).toBeTruthy());
    for (const template of WORKFLOW_TEMPLATES) {
      expect(screen.getByText(template.label)).toBeTruthy();
    }
    // …and the existing workflows, as clone sources.
    await waitFor(() => expect(screen.getByText('Standard')).toBeTruthy());
  });

  it('opens the builder on the picked shape, with no workflow id yet', async () => {
    render(
      <NavigationProvider>
        <WorkflowBuilderScreen workflowId={null} onBack={() => {}} />
      </NavigationProvider>,
    );
    await waitFor(() => expect(screen.getByTestId('template-picker')).toBeTruthy());

    fireEvent.click(screen.getByText('Plan → Implement → Validate'));
    await waitFor(() => expect(screen.getByTestId('workflow-builder')).toBeTruthy());

    // A workflow that doesn't exist yet has no version history to offer.
    expect(screen.queryByLabelText('Version history')).toBeNull();
  });

  it('creates the workflow on first save (workflowId travels as null)', async () => {
    render(
      <NavigationProvider>
        <WorkflowBuilderScreen workflowId={null} onBack={() => {}} />
      </NavigationProvider>,
    );
    await waitFor(() => expect(screen.getByTestId('template-picker')).toBeTruthy());
    fireEvent.click(screen.getByText('Plan → Implement → Validate'));
    await waitFor(() => expect(screen.getByTestId('workflow-builder')).toBeTruthy());

    fireEvent.change(screen.getByLabelText('Workflow name'), {
      target: { value: 'My pipeline' },
    });
    fireEvent.click(screen.getByTitle(/Save a new version/));
    await waitFor(() => expect(saved.length).toBe(1));
    expect(saved[0].workflowId).toBeNull();
    expect(saved[0].name).toBe('My pipeline');
  });

  it('clones an existing workflow as a new, unsaved one', async () => {
    render(
      <NavigationProvider>
        <WorkflowBuilderScreen workflowId={null} onBack={() => {}} />
      </NavigationProvider>,
    );
    await waitFor(() => expect(screen.getByText('Standard')).toBeTruthy());

    fireEvent.click(screen.getByText('Standard'));
    await waitFor(() => expect(screen.getByTestId('workflow-builder')).toBeTruthy());

    // Named as a copy, and detached from the source's history — saving must
    // never append a version to the workflow that was cloned.
    expect((screen.getByLabelText('Workflow name') as HTMLInputElement).value).toBe(
      'Bugfix (copy)',
    );
    fireEvent.click(screen.getByTitle(/Save a new version/));
    await waitFor(() => expect(saved.length).toBe(1));
    expect(saved[0].workflowId).toBeNull();
  });
});

describe('templates', () => {
  it('give every agent node a prompt, so the shape is savable as authored', () => {
    for (const template of WORKFLOW_TEMPLATES) {
      const def = template.build();
      for (const node of def.nodes) {
        if (node.type !== 'agent') continue;
        const prompt = (node.config as Record<string, unknown> | undefined)?.prompt_template;
        expect(typeof prompt === 'string' && prompt.trim().length > 0).toBe(true);
      }
    }
  });

  it('wire the three-step shape as a chain ending in finalize', () => {
    const def = templateById('plan-implement-validate')!.build();
    expect(def.nodes.map((n) => n.id)).toEqual([
      's-plan',
      's-implement',
      's-validate',
      's-finalize',
    ]);
    expect(def.edges).toHaveLength(3);
    // Exactly one finalize, and it is a sink — the two lint rules that would
    // otherwise block a save of an untouched template.
    expect(def.nodes.filter((n) => n.type === 'finalize')).toHaveLength(1);
    expect(def.edges.some((e) => e.from === 's-finalize')).toBe(false);
  });

  it('pre-wire the validate loop the bundled starters use', () => {
    const def = templateById('plan-implement-validate')!.build();
    const validate = def.nodes.find((n) => n.id === 's-validate')!;
    expect(validate.retry?.verdict?.strategy).toBe('redirect');
    expect(validate.retry?.verdict?.redirect_to).toBe('s-implement');
  });

  it('leave the blank shape genuinely blank', () => {
    const def = templateById('blank')!.build();
    expect(def.nodes).toHaveLength(0);
    expect(def.edges).toHaveLength(0);
  });
});
