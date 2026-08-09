// Behavior tests for the SegmentedControl primitive (UI_REDESIGN_PLAN §5.1).
//
// The load-bearing parts are the ones a later migration could silently break:
// the radiogroup/aria-checked contract, roving tabindex plus arrow movement,
// the callback carrying the option's own typed value, and the forwarded ref
// (`useRunColumnLayout` measures the element RunViewToggle hands it).

import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { List, Network } from 'lucide-react';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { SegmentedControl, type SegmentedOption } from './SegmentedControl';

type Filter = 'all' | 'needs-you' | 'active' | 'done';

const FILTERS: readonly SegmentedOption<Filter>[] = [
  { value: 'all', label: 'All' },
  { value: 'needs-you', label: 'Needs you', count: 2, countTone: 'amber' },
  { value: 'active', label: 'Active' },
  { value: 'done', label: 'Done' },
];

function renderFilters(value: Filter, onChange: (next: Filter) => void) {
  return render(
    <SegmentedControl options={FILTERS} value={value} onChange={onChange} ariaLabel="Filter pipelines" />,
  );
}

describe('SegmentedControl', () => {
  it('renders a named radiogroup with one radio per option', () => {
    renderFilters('all', () => {});

    const group = screen.getByRole('radiogroup', { name: 'Filter pipelines' });
    expect(group).toBeInTheDocument();
    expect(screen.getAllByRole('radio')).toHaveLength(FILTERS.length);
  });

  it('marks only the selected option as checked', () => {
    renderFilters('active', () => {});

    expect(screen.getByRole('radio', { name: /Active/ })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('radio', { name: /All/ })).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByRole('radio', { name: /Done/ })).toHaveAttribute('aria-checked', 'false');
  });

  it('keeps a single tab stop on the selected option', () => {
    renderFilters('done', () => {});

    expect(screen.getByRole('radio', { name: /Done/ })).toHaveAttribute('tabindex', '0');
    expect(screen.getByRole('radio', { name: /All/ })).toHaveAttribute('tabindex', '-1');
  });

  it('falls back to a tab stop on the first option when the value matches none', () => {
    render(
      <SegmentedControl
        options={FILTERS}
        value={'gone' as Filter}
        onChange={() => {}}
        ariaLabel="Filter pipelines"
      />,
    );

    expect(screen.getByRole('radio', { name: /All/ })).toHaveAttribute('tabindex', '0');
    expect(screen.queryByRole('radio', { checked: true })).not.toBeInTheDocument();
  });

  it('selects the next option and moves focus on ArrowRight', async () => {
    const onChange = vi.fn<(next: Filter) => void>();
    renderFilters('all', onChange);

    screen.getByRole('radio', { name: /All/ }).focus();
    await userEvent.keyboard('{ArrowRight}');

    expect(onChange).toHaveBeenCalledExactlyOnceWith('needs-you');
    expect(screen.getByRole('radio', { name: /Needs you/ })).toHaveFocus();
  });

  it('wraps from the last option to the first on ArrowRight', async () => {
    const onChange = vi.fn<(next: Filter) => void>();
    renderFilters('done', onChange);

    screen.getByRole('radio', { name: /Done/ }).focus();
    await userEvent.keyboard('{ArrowRight}');

    expect(onChange).toHaveBeenCalledExactlyOnceWith('all');
  });

  it('walks backwards on ArrowLeft and ArrowUp, wrapping to the last option', async () => {
    const onChange = vi.fn<(next: Filter) => void>();
    renderFilters('all', onChange);

    screen.getByRole('radio', { name: /All/ }).focus();
    await userEvent.keyboard('{ArrowLeft}');
    expect(onChange).toHaveBeenLastCalledWith('done');

    await userEvent.keyboard('{ArrowUp}');
    expect(onChange).toHaveBeenLastCalledWith('done');
  });

  it('jumps to the ends on Home and End', async () => {
    const onChange = vi.fn<(next: Filter) => void>();
    renderFilters('active', onChange);

    screen.getByRole('radio', { name: /Active/ }).focus();
    await userEvent.keyboard('{End}');
    expect(onChange).toHaveBeenLastCalledWith('done');

    await userEvent.keyboard('{Home}');
    expect(onChange).toHaveBeenLastCalledWith('all');
  });

  it('ignores keys it does not own', async () => {
    const onChange = vi.fn<(next: Filter) => void>();
    renderFilters('all', onChange);

    screen.getByRole('radio', { name: /All/ }).focus();
    await userEvent.keyboard('{Tab}x');

    expect(onChange).not.toHaveBeenCalled();
  });

  it('reports the clicked option value', async () => {
    const onChange = vi.fn<(next: Filter) => void>();
    renderFilters('all', onChange);

    await userEvent.click(screen.getByRole('radio', { name: /Needs you/ }));

    expect(onChange).toHaveBeenCalledExactlyOnceWith('needs-you');
  });

  it('does not re-report the option that is already selected', async () => {
    const onChange = vi.fn<(next: Filter) => void>();
    renderFilters('all', onChange);

    await userEvent.click(screen.getByRole('radio', { name: /All/ }));

    expect(onChange).not.toHaveBeenCalled();
  });

  it('renders a count badge only when the count is above zero', () => {
    const { rerender } = renderFilters('all', () => {});
    expect(screen.getByText('2')).toBeInTheDocument();

    rerender(
      <SegmentedControl
        options={[
          { value: 'all', label: 'All', count: 0 },
          { value: 'done', label: 'Done' },
        ]}
        value="all"
        onChange={() => {}}
        ariaLabel="Filter pipelines"
      />,
    );
    expect(screen.queryByText('0')).not.toBeInTheDocument();
  });

  it('keeps the count out of the accessible name it appends to', () => {
    renderFilters('all', () => {});

    expect(screen.getByRole('radio', { name: 'Needs you, 2' })).toBeInTheDocument();
  });

  it('forwards its ref to the group element', () => {
    const ref = createRef<HTMLDivElement>();
    render(
      <SegmentedControl
        options={FILTERS}
        value="all"
        onChange={() => {}}
        ariaLabel="Filter pipelines"
        ref={ref}
      />,
    );

    expect(ref.current).toBeInstanceOf(HTMLDivElement);
    expect(ref.current).toBe(screen.getByRole('radiogroup'));
    expect(ref.current?.contains(screen.getByRole('radio', { name: /All/ }))).toBe(true);
  });

  it('accepts a callback ref, as useRunColumnLayout supplies', () => {
    const seen: Array<HTMLDivElement | null> = [];
    render(
      <SegmentedControl
        options={[
          { value: 'graph', label: 'Graph', icon: Network },
          { value: 'timeline', label: 'Timeline', icon: List },
        ]}
        value="graph"
        onChange={() => {}}
        ariaLabel="Run view"
        ref={(el) => {
          seen.push(el);
        }}
      />,
    );

    expect(seen[0]).toBeInstanceOf(HTMLDivElement);
  });

  it('renders the run-toolbar density by default and a tighter one on request', () => {
    const { rerender } = renderFilters('all', () => {});

    const group = screen.getByRole('radiogroup');
    expect(group).toHaveAttribute('data-size', 'md');
    expect(screen.getByRole('radio', { name: /All/ }).className).toContain('px-3 py-1.5');

    rerender(
      <SegmentedControl
        options={FILTERS}
        value="all"
        onChange={() => {}}
        ariaLabel="Filter pipelines"
        size="sm"
      />,
    );
    expect(screen.getByRole('radiogroup')).toHaveAttribute('data-size', 'sm');
    expect(screen.getByRole('radio', { name: /All/ }).className).not.toContain('px-3 py-1.5');
  });

  it('keeps the measurable group element when it has no options', () => {
    const onChange = vi.fn<(next: Filter) => void>();
    render(<SegmentedControl options={[]} value="all" onChange={onChange} ariaLabel="Filter pipelines" />);

    const group = screen.getByRole('radiogroup');
    expect(screen.queryAllByRole('radio')).toHaveLength(0);

    fireEvent.keyDown(group, { key: 'ArrowRight' });
    expect(onChange).not.toHaveBeenCalled();
  });
});
