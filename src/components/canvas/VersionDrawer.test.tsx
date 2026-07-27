/**
 * `VersionDrawer` (task P3.4) — the drawer's own contract, driven through the
 * mocked Tauri boundary:
 *
 *  - the immutable `workflow_versions` rows are listed newest-first,
 *  - comparing fetches the *named* version's graph and hands a comparison up,
 *  - restore writes through `workflow_restore_version` (append, never edit) and
 *    reports what landed,
 *  - and both writes are refused while the editor is dirty, with the reason
 *    visible rather than a silently dead button.
 */
import { render, screen, cleanup, fireEvent, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke, type InvokeArgs } from '@tauri-apps/api/core';

import {
  VersionDrawer,
  type RestoredWorkflow,
  type VersionComparison,
} from './VersionDrawer';
import type { WorkflowDefinitionV2 } from './types';

function graph(nodeIds: string[]): WorkflowDefinitionV2 {
  return {
    schema_version: 2,
    id: 'wf-1',
    name: 'History Test',
    nodes: nodeIds.map((id) => ({ id, type: 'agent', title: id, position: { x: 0, y: 0 } })),
    edges: [],
  };
}

const ROWS = [
  {
    id: 'wf-1-v1',
    workflow_id: 'wf-1',
    version: 1,
    steps_json: '[]',
    note: 'Initial version',
    created_at: Date.now() - 86_400_000,
  },
  {
    id: 'wf-1-v2',
    workflow_id: 'wf-1',
    version: 2,
    steps_json: '[]',
    note: 'Added a critic',
    created_at: Date.now() - 3_600_000,
  },
];

/** Graphs the mocked `workflow_version_graph` serves, per version id. */
const GRAPHS: Record<string, WorkflowDefinitionV2> = {
  'wf-1-v1': graph(['plan']),
  'wf-1-v2': graph(['plan', 'critic']),
  'wf-1-v3': graph(['plan']),
};

let restoreResult = {
  name: 'History Test',
  description: 'seeded',
  version: 3,
  version_id: 'wf-1-v3',
};

function mockBackend() {
  vi.mocked(invoke).mockImplementation((cmd: string, args?: InvokeArgs) => {
    switch (cmd) {
      case 'workflow_versions':
        return Promise.resolve(ROWS);
      case 'workflow_version_graph':
        return Promise.resolve(GRAPHS[String((args as Record<string, string>).versionId)]);
      case 'workflow_restore_version':
      case 'workflow_revert_to_default':
        return Promise.resolve(restoreResult);
      default:
        return Promise.resolve(undefined);
    }
  });
}

interface HarnessProps {
  dirty?: boolean;
  isStarter?: boolean;
  comparison?: VersionComparison | null;
  onCompare?: (c: VersionComparison | null) => void;
  onRestored?: (r: RestoredWorkflow) => void;
}

function renderDrawer({
  dirty = false,
  isStarter = false,
  comparison = null,
  onCompare = () => {},
  onRestored = () => {},
}: HarnessProps = {}) {
  return render(
    <VersionDrawer
      workflowId="wf-1"
      isStarter={isStarter}
      dirty={dirty}
      comparison={comparison}
      onCompare={onCompare}
      onRestored={onRestored}
      onClose={() => {}}
    />,
  );
}

beforeEach(() => {
  restoreResult = {
    name: 'History Test',
    description: 'seeded',
    version: 3,
    version_id: 'wf-1-v3',
  };
  mockBackend();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('VersionDrawer', () => {
  it('lists versions newest-first and marks the current one', async () => {
    renderDrawer();
    const rows = await screen.findAllByTestId(/^version-row-/);
    expect(rows.map((r) => r.dataset.testid)).toEqual(['version-row-2', 'version-row-1']);
    expect(screen.getByText('Added a critic')).toBeInTheDocument();
    // Only the highest version is "Current".
    expect(screen.getAllByText('Current')).toHaveLength(1);
    expect(rows[0]).toHaveTextContent('Current');
  });

  it('hands up a comparison built from the named version’s graph', async () => {
    const onCompare = vi.fn();
    renderDrawer({ onCompare });
    await screen.findByTestId('version-row-1');

    fireEvent.click(
      within(screen.getByTestId('version-row-1')).getByRole('button', { name: 'Compare' }),
    );

    await waitFor(() => expect(onCompare).toHaveBeenCalled());
    const comparison = onCompare.mock.calls[0][0] as VersionComparison;
    expect(comparison.from.versionId).toBe('wf-1-v1');
    expect(comparison.from.label).toBe('v1');
    expect(comparison.from.graph.nodes.map((n) => n.id)).toEqual(['plan']);
    // The working copy is the builder's to supply, so it comes through null.
    expect(comparison.to).toEqual({ versionId: null, label: 'Working copy', graph: null });
  });

  it('compares two stored versions when the target is changed', async () => {
    const onCompare = vi.fn();
    renderDrawer({ onCompare });
    await screen.findByTestId('version-row-1');

    fireEvent.change(screen.getByLabelText('Compare against'), {
      target: { value: 'wf-1-v2' },
    });
    fireEvent.click(
      within(screen.getByTestId('version-row-1')).getByRole('button', { name: 'Compare' }),
    );

    await waitFor(() => expect(onCompare).toHaveBeenCalled());
    const comparison = onCompare.mock.calls[onCompare.mock.calls.length - 1][0] as VersionComparison;
    expect(comparison.from.graph.nodes.map((n) => n.id)).toEqual(['plan']);
    expect(comparison.to.label).toBe('v2');
    expect(comparison.to.graph?.nodes.map((n) => n.id)).toEqual(['plan', 'critic']);
  });

  it('restores a version and reports the new one that landed', async () => {
    const onRestored = vi.fn();
    renderDrawer({ onRestored });
    await screen.findByTestId('version-row-1');

    fireEvent.click(screen.getByTitle('Restore v1 as a new version'));

    await waitFor(() => expect(onRestored).toHaveBeenCalled());
    expect(invoke).toHaveBeenCalledWith('workflow_restore_version', {
      workflowId: 'wf-1',
      versionId: 'wf-1-v1',
    });
    expect(onRestored.mock.calls[0][0]).toMatchObject({
      kind: 'restore',
      version: 3,
      versionId: 'wf-1-v3',
      sourceVersion: 1,
    });
  });

  it('refuses to restore over unsaved edits, and says why', async () => {
    const onRestored = vi.fn();
    renderDrawer({ dirty: true, onRestored });
    await screen.findByTestId('version-row-1');

    // Every row is blocked, not just the one that would have been allowed.
    const blocked = screen.getAllByTitle(/Save or discard your unsaved edits first/);
    expect(blocked).toHaveLength(ROWS.length);
    const restore = blocked[0];
    expect(restore).toBeDisabled();
    // The explanation is on the card, not only in a tooltip nobody hovers.
    expect(
      screen.getAllByText(/Save or discard your unsaved edits first/).length,
    ).toBeGreaterThan(0);

    fireEvent.click(restore);
    expect(onRestored).not.toHaveBeenCalled();
  });

  it('never offers to restore the version already loaded', async () => {
    renderDrawer();
    await screen.findByTestId('version-row-2');
    const latestRestore = screen.getByTitle('This is the current version.');
    expect(latestRestore).toBeDisabled();
  });

  it('offers revert-to-default only for starters', async () => {
    const { unmount } = renderDrawer();
    await screen.findByTestId('version-row-1');
    expect(screen.queryByText('Revert to default')).not.toBeInTheDocument();
    unmount();

    const onRestored = vi.fn();
    renderDrawer({ isStarter: true, onRestored });
    await screen.findByTestId('version-row-1');
    fireEvent.click(screen.getByText('Revert to default'));

    await waitFor(() => expect(onRestored).toHaveBeenCalled());
    expect(invoke).toHaveBeenCalledWith('workflow_revert_to_default', { workflowId: 'wf-1' });
    expect(onRestored.mock.calls[0][0]).toMatchObject({ kind: 'revert', version: 3 });
  });

  it('surfaces a failed restore instead of pretending it worked', async () => {
    const onRestored = vi.fn();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === 'workflow_versions') return Promise.resolve(ROWS);
      if (cmd === 'workflow_restore_version') return Promise.reject(new Error('db is locked'));
      return Promise.resolve(undefined);
    });
    renderDrawer({ onRestored });
    await screen.findByTestId('version-row-1');

    fireEvent.click(screen.getByTitle('Restore v1 as a new version'));

    expect(await screen.findByText(/db is locked/)).toBeInTheDocument();
    expect(onRestored).not.toHaveBeenCalled();
  });
});
