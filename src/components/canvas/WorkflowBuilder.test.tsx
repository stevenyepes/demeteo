/**
 * `WorkflowBuilder` (task P3.3) — the three Done-when claims, each asserted end
 * to end through the real canvas, palette, and config panel:
 *
 *  - *"invalid save is impossible (error toast names findings)"* — an error
 *    finding disables Save, lists its reason, and makes even the `⌘S` path
 *    refuse with an explanation instead of writing.
 *  - *"refresh mid-edit restores draft"* — a draft left in `localStorage` by a
 *    previous session is offered and restores onto the canvas.
 *  - *"⌘Z / ⇧⌘Z work across node+edge ops"* — undo/redo over a palette add.
 *
 * Plus the dirty guard itself (audit F38), driven through the *real*
 * `NavigationProvider` so the test proves a `navigate()` from elsewhere in the
 * app is blocked — the whole point of guarding the context rather than a
 * component's own Back button.
 */
import {
  render,
  screen,
  cleanup,
  fireEvent,
  waitFor,
  within,
  act,
} from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke, type InvokeArgs } from '@tauri-apps/api/core';

import { NavigationProvider, useNavigation } from '../../context/NavigationContext';
import { WorkflowBuilder } from './WorkflowBuilder';
import catalogFixture from './__fixtures__/node_catalog.json';
import { resetNodeTypeCache, type NodeTypeInfo } from './nodeCatalog';
import { draftKey, loadDraft } from './workflowDraft';
import type { LintFinding } from './lint';
import type { WorkflowDefinitionV2 } from './types';

const CATALOG = catalogFixture as unknown as NodeTypeInfo[];

const DEF: WorkflowDefinitionV2 = {
  schema_version: 2,
  id: 'wf-b',
  name: 'Builder Test',
  nodes: [
    {
      id: 'plan',
      type: 'agent',
      title: 'Research Codebase',
      config: { prompt_template: 'research it' },
      position: { x: 0, y: 0 },
    },
    { id: 'ship', type: 'finalize', title: 'Publish Branch', position: { x: 0, y: 160 } },
  ],
  edges: [{ from: 'plan', to: 'ship' }],
};

/** Stored history for the version drawer: v1 predates the finalize node. */
const VERSION_ROWS = [
  {
    id: 'wf-b-v1',
    workflow_id: 'wf-b',
    version: 1,
    steps_json: '[]',
    note: 'Initial version',
    created_at: 1,
  },
  {
    id: 'wf-b-v3',
    workflow_id: 'wf-b',
    version: 3,
    steps_json: '[]',
    note: 'Added the finalize step',
    created_at: 3,
  },
];

const VERSION_GRAPHS: Record<string, WorkflowDefinitionV2> = {
  'wf-b-v1': { ...DEF, nodes: [DEF.nodes[0]], edges: [] },
  'wf-b-v3': DEF,
  // What a restore of v1 lands as.
  'wf-b-v4': { ...DEF, nodes: [DEF.nodes[0]], edges: [] },
};

/** Findings the mocked `workflow_lint` returns; per-test. */
let findings: LintFinding[] = [];

function mockBackend() {
  vi.mocked(invoke).mockImplementation((cmd: string, args?: InvokeArgs) => {
    switch (cmd) {
      case 'node_types_list':
        return Promise.resolve(CATALOG);
      case 'workflow_lint':
        return Promise.resolve(findings);
      case 'list_agents':
        return Promise.resolve([]);
      // Version history (P3.4). v1 is the graph without the finalize node, so
      // comparing it against the working copy has something to report.
      case 'workflow_versions':
        return Promise.resolve(VERSION_ROWS);
      case 'workflow_version_graph':
        return Promise.resolve(VERSION_GRAPHS[String((args as Record<string, string>).versionId)]);
      case 'workflow_restore_version':
        return Promise.resolve({
          name: 'Builder Test',
          description: '',
          version: 4,
          version_id: 'wf-b-v4',
        });
      default:
        return Promise.resolve(undefined);
    }
  });
}

interface HarnessProps {
  onSave?: (req: { definition: WorkflowDefinitionV2; name: string }) => Promise<void>;
  onClose?: () => void;
  workflowId?: string | null;
}

/** The builder inside a real navigation context, plus a button that navigates
 *  from "elsewhere in the app" so the guard can be exercised honestly. */
function Harness({ onSave = () => Promise.resolve(), onClose = () => {}, workflowId = 'wf-b' }: HarnessProps) {
  const { view, navigate } = useNavigation();
  return (
    <div style={{ width: 900, height: 700 }}>
      <span data-testid="view">{view.kind}</span>
      <button type="button" onClick={() => navigate({ kind: 'workflows' })}>
        Elsewhere
      </button>
      <WorkflowBuilder
        workflowId={workflowId}
        definition={DEF}
        name="Builder Test"
        version={3}
        onSave={onSave}
        onClose={onClose}
      />
    </div>
  );
}

function renderBuilder(props: HarnessProps = {}) {
  return render(
    <NavigationProvider>
      <Harness {...props} />
    </NavigationProvider>,
  );
}

/** Wait for the palette (i.e. the node catalog) and the first lint to land. */
async function ready() {
  await screen.findByTestId('node-palette');
  await waitFor(() => expect(screen.getByTestId('lint-status')).toBeInTheDocument());
}

beforeAll(() => {
  // React Flow needs a handful of browser APIs jsdom lacks — same set the
  // P2.1 canvas test stubs.
  class DOMMatrixStub {
    m22 = 1;
  }
  vi.stubGlobal('DOMMatrixReadOnly', DOMMatrixStub);
  vi.stubGlobal('DOMMatrix', DOMMatrixStub);
  Object.defineProperty(HTMLElement.prototype, 'getBoundingClientRect', {
    configurable: true,
    value: () => ({
      x: 0,
      y: 0,
      width: 900,
      height: 700,
      top: 0,
      left: 0,
      right: 900,
      bottom: 700,
      toJSON: () => {},
    }),
  });
});

beforeEach(() => {
  findings = [];
  localStorage.clear();
  resetNodeTypeCache();
  mockBackend();
});

afterEach(() => {
  cleanup();
  vi.mocked(invoke).mockReset();
});

describe('version history (P3.4)', () => {
  /** Open the drawer and wait for its rows. */
  async function openHistory() {
    fireEvent.click(screen.getByRole('button', { name: 'Version history' }));
    await screen.findByTestId('version-row-1');
  }

  it('renders the diff between a stored version and the working copy', async () => {
    renderBuilder();
    await ready();
    await openHistory();

    const v1 = screen.getByTestId('version-row-1');
    fireEvent.click(within(v1).getByRole('button', { name: 'Compare' }));

    const banner = await screen.findByTestId('compare-banner');
    expect(banner).toHaveTextContent('Comparing v1 → Working copy');
    expect(banner).toHaveTextContent('1 added');

    // The finalize node the working copy has and v1 didn't is marked on the
    // canvas itself — the Done-when for this task.
    expect(await screen.findByTestId('node-diff-added')).toBeInTheDocument();
    // …and the merged graph is read-only while comparing: no palette, no
    // config panel to edit a version that no longer exists.
    expect(screen.queryByTestId('node-palette')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Exit compare' }));
    await waitFor(() => expect(screen.getByTestId('node-palette')).toBeInTheDocument());
    expect(screen.queryByTestId('node-diff-added')).not.toBeInTheDocument();
  });

  it('adopts a restored version: new graph, new version, clean editor', async () => {
    const onWorkflowReplaced = vi.fn();
    render(
      <NavigationProvider>
        <div style={{ width: 900, height: 700 }}>
          <WorkflowBuilder
            workflowId="wf-b"
            definition={DEF}
            name="Builder Test"
            version={3}
            onSave={() => Promise.resolve()}
            onWorkflowReplaced={onWorkflowReplaced}
            onClose={() => {}}
          />
        </div>
      </NavigationProvider>,
    );
    await ready();
    // Dirty the editor, then clean it, so the restore isn't fighting the guard.
    await openHistory();

    fireEvent.click(screen.getByTitle('Restore v1 as a new version'));

    await waitFor(() => expect(onWorkflowReplaced).toHaveBeenCalledWith({
      version: 4,
      name: 'Builder Test',
      description: '',
    }));
    // The canvas now shows the restored graph…
    await waitFor(() =>
      expect(screen.queryByText('Publish Branch')).not.toBeInTheDocument(),
    );
    // …the header follows the version that landed, and nothing is unsaved:
    // the restore is already a stored version.
    expect(screen.getByRole('button', { name: 'Version history' })).toHaveTextContent('v4');
    expect(screen.queryByTestId('dirty-indicator')).not.toBeInTheDocument();
  });

  it('refuses to restore over unsaved edits', async () => {
    renderBuilder();
    await ready();
    fireEvent.click(screen.getByRole('option', { name: /Gate/ }));
    expect(screen.getByTestId('dirty-indicator')).toBeInTheDocument();

    await openHistory();
    for (const button of screen.getAllByTitle(/Save or discard your unsaved edits/)) {
      expect(button).toBeDisabled();
    }
    expect(invoke).not.toHaveBeenCalledWith('workflow_restore_version', expect.anything());
  });
});

describe('save gating', () => {
  it('saves a clean graph and comes back clean', async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    renderBuilder({ onSave });
    await ready();

    // An edit through the real palette: click-to-add is the a11y path.
    fireEvent.click(screen.getByRole('option', { name: /Gate/ }));
    expect(screen.getByTestId('dirty-indicator')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));

    const saved = onSave.mock.calls[0][0] as { definition: WorkflowDefinitionV2 };
    expect(saved.definition.nodes.map((n) => n.type)).toContain('gate');
    // Saving is what makes it clean again.
    await waitFor(() =>
      expect(screen.queryByTestId('dirty-indicator')).not.toBeInTheDocument(),
    );
  });

  it('refuses to save while an error finding stands, and names it', async () => {
    findings = [
      {
        severity: 'error',
        code: 'missing-prompt',
        node: 'plan',
        message: "agent node 'plan' has no prompt_template",
      },
    ];
    const onSave = vi.fn().mockResolvedValue(undefined);
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    renderBuilder({ onSave });
    await ready();

    // The reason is on screen, by node title, not hidden in a tooltip.
    const reasons = await screen.findByTestId('lint-errors');
    expect(reasons).toHaveTextContent('Research Codebase');
    expect(reasons).toHaveTextContent('no prompt_template');
    expect(screen.getByTestId('lint-status')).toHaveTextContent('1 error');
    expect(screen.getByRole('button', { name: /^Save$/ })).toBeDisabled();

    // The shortcut bypasses the disabled button, so it must refuse loudly.
    fireEvent.keyDown(document, { key: 's', metaKey: true });
    await waitFor(() => expect(warn).toHaveBeenCalled());
    expect(onSave).not.toHaveBeenCalled();
    expect(warn.mock.calls.some((c) => String(c[0]).includes('Cannot save'))).toBe(true);
    warn.mockRestore();
  });

  it('badges the accused node on the canvas', async () => {
    findings = [
      {
        severity: 'warning',
        code: 'dead-end',
        node: 'plan',
        message: "node 'plan' is a sink but not the finalize node",
      },
    ];
    renderBuilder();
    await ready();
    await waitFor(() => expect(screen.getByTestId('node-lint-warning')).toBeInTheDocument());
    expect(screen.queryByTestId('node-lint-error')).not.toBeInTheDocument();
    // A warning is an observation: the save stays open.
    expect(screen.getByRole('button', { name: /^Save$/ })).not.toBeDisabled();
  });
});

/** Node cards currently on the canvas. */
function nodeCount(): number {
  return document.querySelectorAll('.react-flow__node').length;
}

describe('undo / redo', () => {
  it('⌘Z reverts a node add and ⇧⌘Z restores it', async () => {
    renderBuilder();
    await ready();
    await waitFor(() => expect(nodeCount()).toBe(DEF.nodes.length));

    fireEvent.click(screen.getByRole('option', { name: /Sync/ }));
    await waitFor(() => expect(nodeCount()).toBe(DEF.nodes.length + 1));

    fireEvent.keyDown(document, { key: 'z', metaKey: true });
    await waitFor(() => expect(nodeCount()).toBe(DEF.nodes.length));
    expect(screen.queryByTestId('dirty-indicator')).not.toBeInTheDocument();

    fireEvent.keyDown(document, { key: 'z', metaKey: true, shiftKey: true });
    await waitFor(() => expect(nodeCount()).toBe(DEF.nodes.length + 1));
  });

  it('leaves ⌘Z to the text field the author is typing in', async () => {
    renderBuilder();
    await ready();
    fireEvent.click(screen.getByRole('option', { name: /Sync/ }));
    await waitFor(() => expect(nodeCount()).toBe(DEF.nodes.length + 1));

    fireEvent.keyDown(screen.getByLabelText('Workflow name'), { key: 'z', metaKey: true });
    // The graph edit is untouched: the browser's own undo owns that keystroke.
    expect(nodeCount()).toBe(DEF.nodes.length + 1);
  });
});

describe('dirty guard (audit F38)', () => {
  it('blocks a navigation from anywhere in the app until resolved', async () => {
    renderBuilder();
    await ready();
    fireEvent.click(screen.getByRole('option', { name: /Gate/ }));

    fireEvent.click(screen.getByRole('button', { name: 'Elsewhere' }));
    expect(screen.getByTestId('dirty-guard')).toBeInTheDocument();
    // Still here: the intent was vetoed, not deferred.
    expect(screen.getByTestId('view')).toHaveTextContent('empty-state');

    // Keep editing → the intent is dropped entirely.
    fireEvent.click(screen.getByRole('button', { name: 'Keep editing' }));
    expect(screen.queryByTestId('dirty-guard')).not.toBeInTheDocument();
    expect(screen.getByTestId('view')).toHaveTextContent('empty-state');

    // Discard → the blocked navigation is replayed.
    fireEvent.click(screen.getByRole('button', { name: 'Elsewhere' }));
    fireEvent.click(screen.getByRole('button', { name: 'Discard' }));
    await waitFor(() => expect(screen.getByTestId('view')).toHaveTextContent('workflows'));
  });

  it('saves and then leaves when asked to', async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    renderBuilder({ onSave });
    await ready();
    fireEvent.click(screen.getByRole('option', { name: /Gate/ }));

    fireEvent.click(screen.getByRole('button', { name: 'Elsewhere' }));
    fireEvent.click(screen.getByRole('button', { name: 'Save and leave' }));

    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getByTestId('view')).toHaveTextContent('workflows'));
  });

  it('lets a clean editor leave without a prompt', async () => {
    renderBuilder();
    await ready();
    fireEvent.click(screen.getByRole('button', { name: 'Elsewhere' }));
    expect(screen.queryByTestId('dirty-guard')).not.toBeInTheDocument();
    expect(screen.getByTestId('view')).toHaveTextContent('workflows');
  });

  it('guards its own Back arrow too', async () => {
    const onClose = vi.fn();
    renderBuilder({ onClose });
    await ready();
    fireEvent.click(screen.getByRole('option', { name: /Gate/ }));

    fireEvent.click(screen.getByRole('button', { name: 'Back to workflows' }));
    expect(screen.getByTestId('dirty-guard')).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Discard' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe('draft recovery', () => {
  const stored = {
    format: 1,
    workflowId: 'wf-b',
    name: 'Builder Test',
    description: '',
    savedAt: 1_700_000_000_000,
    definition: {
      ...DEF,
      nodes: [...DEF.nodes, { id: 'gate', type: 'gate', title: 'Recovered Gate' }],
    },
  };

  it('offers a recovered draft and restores it onto the canvas', async () => {
    localStorage.setItem(draftKey('wf-b'), JSON.stringify(stored));
    renderBuilder();
    await ready();

    const banner = await screen.findByTestId('draft-offer');
    expect(banner).toHaveTextContent('unsaved draft');
    // Not applied until asked — the loaded version is still what's rendered.
    expect(screen.queryByText('Recovered Gate')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Restore' }));
    await waitFor(() => expect(screen.getByText('Recovered Gate')).toBeInTheDocument());
    // Restored work is unsaved work.
    expect(screen.getByTestId('dirty-indicator')).toBeInTheDocument();
    expect(screen.queryByTestId('draft-offer')).not.toBeInTheDocument();
  });

  it('discarding the offer drops the stored draft', async () => {
    localStorage.setItem(draftKey('wf-b'), JSON.stringify(stored));
    renderBuilder();
    await screen.findByTestId('draft-offer');

    fireEvent.click(screen.getByRole('button', { name: 'Discard' }));
    expect(loadDraft('wf-b')).toBeNull();
    expect(screen.queryByTestId('draft-offer')).not.toBeInTheDocument();
  });

  it('does not dangle a draft that matches the saved definition', async () => {
    localStorage.setItem(
      draftKey('wf-b'),
      JSON.stringify({ ...stored, definition: DEF }),
    );
    renderBuilder();
    await ready();
    expect(screen.queryByTestId('draft-offer')).not.toBeInTheDocument();
    // …and the stale slot is reclaimed.
    expect(loadDraft('wf-b')).toBeNull();
  });

  it('autosaves unsaved work on the 30s cadence', async () => {
    vi.useFakeTimers();
    try {
      renderBuilder();
      // Fake timers still let the catalog/lint promises settle.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      fireEvent.click(screen.getByRole('option', { name: /Gate/ }));
      expect(loadDraft('wf-b')).toBeNull(); // nothing written yet

      await act(async () => {
        await vi.advanceTimersByTimeAsync(30_000);
      });
      const draft = loadDraft('wf-b');
      expect(draft?.definition.nodes.some((n) => n.type === 'gate')).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });
});
