/**
 * The pane switch's one hard contract: exactly one pane in the document.
 *
 * `Inspector` stamps `data-testid="inspector"` and eight assertions across four
 * suites reach for it by that id, which throws on a second match — so a column
 * that kept both panes mounted and hid one with CSS would break them all, and
 * nothing in tsc, biome or the browser would say why.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { InspectorColumn, type InspectorPane } from './InspectorColumn';
import { focusInspectorPane } from './useRunShortcuts';

function mount(pane: InspectorPane, syncBadge = 0, onPaneChange = vi.fn()) {
  const view = render(
    <InspectorColumn
      pane={pane}
      onPaneChange={onPaneChange}
      syncBadge={syncBadge}
      stepInspector={<div data-testid="inspector">the step</div>}
      syncPanel={<div data-testid="sync-panel">the sync</div>}
    />,
  );
  return { ...view, onPaneChange };
}

describe('InspectorColumn', () => {
  it('mounts the step pane alone', () => {
    mount('step');

    expect(screen.getByTestId('inspector')).toBeInTheDocument();
    expect(screen.queryByTestId('sync-panel')).toBeNull();
  });

  it('mounts the sync pane alone', () => {
    mount('sync');

    expect(screen.getByTestId('sync-panel')).toBeInTheDocument();
    expect(screen.queryByTestId('inspector')).toBeNull();
  });

  it('carries what the sync pane is holding on its tab', () => {
    mount('step', 2);
    expect(screen.getByRole('tab', { name: 'Sync · 2' })).toBeInTheDocument();
  });

  it('names the tab plainly when nothing is waiting', () => {
    mount('step', 0);
    expect(screen.getByRole('tab', { name: 'Sync' })).toBeInTheDocument();
  });

  it('switches panes on a click', async () => {
    const { onPaneChange } = mount('step');

    await userEvent.click(screen.getByRole('tab', { name: 'Sync' }));
    expect(onPaneChange).toHaveBeenCalledWith('sync');
  });

  /** `TabBar` moves *and* selects on an arrow key, so the two exclusive-choice
   *  rows in this app do not disagree about what one means. */
  it('switches panes on an arrow key', () => {
    const { onPaneChange } = mount('step');

    fireEvent.keyDown(screen.getByRole('tablist'), { key: 'ArrowRight' });
    expect(onPaneChange).toHaveBeenCalledWith('sync');
  });

  /** Enter aims at the column's roving entry point. With the strip above the
   *  card that is now Step/Sync rather than the step inspector's Overview — the
   *  outermost choice in the column, and a behaviour change worth pinning. */
  it('takes the Enter shortcut onto the pane switch', () => {
    mount('step');
    const column = screen.getByTestId('inspector-column');

    expect(focusInspectorPane(column)).toBe(true);
    expect(document.activeElement).toHaveAccessibleName('Step');
  });
});
