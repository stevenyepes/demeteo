/**
 * `Inspector` (UI_REDESIGN_PLAN §5.1). The header and the body are slots, so
 * what is worth asserting is the part the primitive actually owns: the tab
 * contract. These cover the ARIA wiring a screen reader needs to announce
 * "tab 2 of 4, selected" and the arrow-key navigation that makes the strip one
 * tab stop instead of four.
 */
import { render, screen, cleanup, fireEvent } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useState } from 'react';

import { Inspector } from './Inspector';
import type { TabDef } from './TabBar';

type Tab = 'overview' | 'live' | 'output';

const TABS: readonly TabDef<Tab>[] = [
  { value: 'overview', label: 'Overview' },
  { value: 'live', label: 'Live' },
  { value: 'output', label: 'Output' },
];

function Harness({
  initial = 'overview',
  onTabChange,
}: {
  initial?: Tab;
  onTabChange?: (tab: Tab) => void;
}) {
  const [tab, setTab] = useState<Tab>(initial);
  return (
    <Inspector
      title="Implement Feature"
      ariaLabel="Node detail"
      tabs={TABS}
      activeTab={tab}
      onTabChange={(next) => {
        setTab(next);
        onTabChange?.(next);
      }}
    >
      <div>body: {tab}</div>
    </Inspector>
  );
}

afterEach(cleanup);

describe('Inspector — header', () => {
  it('renders the title, the meta slot and a dismiss control', () => {
    const onDismiss = vi.fn();
    render(
      <Inspector
        title="Implement Feature"
        icon={<span data-testid="icon" />}
        meta={<span>Running</span>}
        onDismiss={onDismiss}
        dismissLabel="Close panel"
        ariaLabel="Node detail"
        tabs={TABS}
        activeTab="overview"
        onTabChange={() => {}}
      >
        <div>body</div>
      </Inspector>,
    );

    expect(screen.getByRole('heading', { name: 'Implement Feature' })).toBeInTheDocument();
    expect(screen.getByTestId('icon')).toBeInTheDocument();
    expect(screen.getByText('Running')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Close panel' }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it('omits the dismiss control when the pane cannot be dismissed', () => {
    render(
      <Inspector
        title="Implement Feature"
        ariaLabel="Node detail"
        tabs={TABS}
        activeTab="overview"
        onTabChange={() => {}}
      >
        <div>body</div>
      </Inspector>,
    );
    expect(screen.queryByRole('button', { name: /close/i })).not.toBeInTheDocument();
  });
});

describe('Inspector — tab a11y contract', () => {
  it('names the tablist and marks exactly the active tab selected', () => {
    render(<Harness initial="live" />);

    expect(screen.getByRole('tablist', { name: 'Node detail' })).toBeInTheDocument();
    const tabs = screen.getAllByRole('tab');
    expect(tabs.map((t) => t.textContent)).toEqual(['Overview', 'Live', 'Output']);
    expect(tabs.map((t) => t.getAttribute('aria-selected'))).toEqual(['false', 'true', 'false']);
  });

  it('points every tab at the panel and labels the panel by the active tab', () => {
    render(<Harness initial="live" />);

    const panel = screen.getByRole('tabpanel');
    const panelId = panel.getAttribute('id');
    expect(panelId).toBeTruthy();
    // Every tab resolves, not just the selected one — the panel element stays
    // mounted across tabs precisely so that holds.
    for (const tab of screen.getAllByRole('tab')) {
      expect(tab.getAttribute('aria-controls')).toBe(panelId);
    }
    expect(panel.getAttribute('aria-labelledby')).toBe(
      screen.getByRole('tab', { name: 'Live' }).getAttribute('id'),
    );
  });

  it('is one tab stop: only the active tab is reachable by Tab', () => {
    render(<Harness initial="live" />);
    expect(screen.getAllByRole('tab').map((t) => t.getAttribute('tabindex'))).toEqual([
      '-1',
      '0',
      '-1',
    ]);
  });

  it('selects on click', () => {
    const onTabChange = vi.fn();
    render(<Harness onTabChange={onTabChange} />);
    fireEvent.click(screen.getByRole('tab', { name: 'Output' }));
    expect(onTabChange).toHaveBeenCalledWith('output');
    expect(screen.getByRole('tabpanel')).toHaveTextContent('body: output');
  });
});

describe('Inspector — keyboard navigation', () => {
  it('moves and selects with the arrow keys, wrapping at both ends', () => {
    render(<Harness />);
    const tablist = screen.getByRole('tablist');

    fireEvent.keyDown(tablist, { key: 'ArrowRight' });
    expect(screen.getByRole('tab', { name: 'Live' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tab', { name: 'Live' })).toHaveFocus();

    fireEvent.keyDown(tablist, { key: 'ArrowLeft' });
    fireEvent.keyDown(tablist, { key: 'ArrowLeft' });
    expect(screen.getByRole('tab', { name: 'Output' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tabpanel')).toHaveTextContent('body: output');

    fireEvent.keyDown(tablist, { key: 'ArrowRight' });
    expect(screen.getByRole('tab', { name: 'Overview' })).toHaveAttribute('aria-selected', 'true');
  });

  it('jumps to the ends with Home and End', () => {
    render(<Harness initial="live" />);
    const tablist = screen.getByRole('tablist');

    fireEvent.keyDown(tablist, { key: 'End' });
    expect(screen.getByRole('tab', { name: 'Output' })).toHaveAttribute('aria-selected', 'true');

    fireEvent.keyDown(tablist, { key: 'Home' });
    expect(screen.getByRole('tab', { name: 'Overview' })).toHaveAttribute('aria-selected', 'true');
  });

  it('leaves other keys to the browser', () => {
    const onTabChange = vi.fn();
    render(<Harness onTabChange={onTabChange} />);
    fireEvent.keyDown(screen.getByRole('tablist'), { key: 'a' });
    expect(onTabChange).not.toHaveBeenCalled();
  });

  it('survives an empty tab list', () => {
    render(
      <Inspector<Tab>
        title="Nothing"
        ariaLabel="Node detail"
        tabs={[]}
        activeTab="overview"
        onTabChange={() => {}}
      >
        <div>body</div>
      </Inspector>,
    );
    fireEvent.keyDown(screen.getByRole('tablist'), { key: 'ArrowRight' });
    expect(screen.queryAllByRole('tab')).toHaveLength(0);
    expect(screen.getByRole('tabpanel')).not.toHaveAttribute('aria-labelledby');
  });
});
