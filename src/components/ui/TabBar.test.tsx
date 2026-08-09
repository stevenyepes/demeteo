/**
 * `TabBar` is exercised through `Inspector` for the panel-owning case, so what
 * is left to cover here is the shape the settings screens use: a strip with no
 * `tabpanel` to point at, at the page-level density.
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useState } from 'react';

import { TabBar, type TabDef } from './TabBar';

type Section = 'general' | 'strategy' | 'memory';

const TABS: readonly TabDef<Section>[] = [
  { value: 'general', label: 'General' },
  { value: 'strategy', label: 'Strategy', icon: <span data-testid="strategy-icon" /> },
  { value: 'memory', label: 'Memory' },
];

function Harness({
  initial = 'general',
  onChange,
}: {
  initial?: Section;
  onChange?: (value: Section) => void;
}) {
  const [active, setActive] = useState<Section>(initial);
  return (
    <TabBar
      tabs={TABS}
      activeTab={active}
      onChange={(next) => {
        setActive(next);
        onChange?.(next);
      }}
      ariaLabel="Project settings sections"
    />
  );
}

afterEach(cleanup);

describe('TabBar', () => {
  it('names the tablist and marks exactly the active tab selected', () => {
    render(<Harness initial="strategy" />);

    expect(screen.getByRole('tablist', { name: 'Project settings sections' })).toBeInTheDocument();
    expect(screen.getAllByRole('tab').map((t) => t.getAttribute('aria-selected'))).toEqual([
      'false',
      'true',
      'false',
    ]);
  });

  it('renders a tab icon beside its label', () => {
    render(<Harness />);
    expect(screen.getByTestId('strategy-icon')).toBeInTheDocument();
  });

  it('points at no panel when the caller renders none', () => {
    render(<Harness />);
    for (const tab of screen.getAllByRole('tab')) {
      expect(tab).not.toHaveAttribute('aria-controls');
      expect(tab).not.toHaveAttribute('id');
    }
  });

  it('is one tab stop and moves with the arrow keys', () => {
    const onChange = vi.fn();
    render(<Harness initial="strategy" onChange={onChange} />);

    expect(screen.getAllByRole('tab').map((t) => t.getAttribute('tabindex'))).toEqual([
      '-1',
      '0',
      '-1',
    ]);

    fireEvent.keyDown(screen.getByRole('tablist'), { key: 'ArrowLeft' });

    expect(onChange).toHaveBeenCalledWith('general');
    expect(screen.getByRole('tab', { name: 'General' })).toHaveFocus();
  });

  it('selects on click', () => {
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);
    fireEvent.click(screen.getByRole('tab', { name: 'Strategy' }));
    expect(onChange).toHaveBeenCalledWith('strategy');
  });

  it('offers the two densities as one strip, not two', () => {
    const { rerender } = render(
      <TabBar tabs={TABS} activeTab="general" onChange={() => {}} ariaLabel="Sections" />,
    );
    const page = screen.getByRole('tablist').getAttribute('data-size');
    const pageSelected = screen.getByRole('tab', { name: 'General' }).className;

    rerender(
      <TabBar
        tabs={TABS}
        activeTab="general"
        onChange={() => {}}
        ariaLabel="Sections"
        size="sm"
      />,
    );
    const dense = screen.getByRole('tablist').getAttribute('data-size');
    const denseSelected = screen.getByRole('tab', { name: 'General' }).className;

    expect([page, dense]).toEqual(['md', 'sm']);
    // Density is the only axis the two sizes differ on: the selected treatment
    // is one decision, so both carry the same accent classes.
    expect(denseSelected).toContain('border-cyan-500');
    expect(pageSelected).toContain('border-cyan-500');
    expect(denseSelected).toContain('text-cyan-400');
    expect(pageSelected).toContain('text-cyan-400');
  });
});
