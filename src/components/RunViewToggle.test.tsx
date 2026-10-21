// The Graph|Timeline toggle, now a thin binding of `SegmentedControl` to the
// run column's view mode.
//
// Two things have to survive that binding: the selected mode still reads as
// selected through the primitive's accent treatment (a local style here is the
// F27 drift the redesign closed), and `onSelect` still speaks `RunViewMode`
// rather than the primitive's generic.
//
// Measurement is no longer this component's: the run view seats it in a chrome
// row beside the density toggle and measures that row, and
// `FeatureDetail.layout.test.tsx` carries the claim that used to live here.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { RunViewToggle } from './RunViewToggle';

describe('RunViewToggle', () => {
  it('offers both run views in a named group', () => {
    render(<RunViewToggle mode="graph" onSelect={() => {}} />);

    expect(screen.getByRole('radiogroup', { name: 'Run view' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /graph/i })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /timeline/i })).toBeInTheDocument();
  });

  it('checks only the active mode', () => {
    render(<RunViewToggle mode="timeline" onSelect={() => {}} />);

    expect(screen.getByRole('radio', { name: /timeline/i })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('radio', { name: /graph/i })).toHaveAttribute('aria-checked', 'false');
  });

  it('dresses the active mode in the shared selected treatment, not a local one', () => {
    render(<RunViewToggle mode="graph" onSelect={() => {}} />);

    expect(screen.getByRole('radio', { name: /graph/i })).toHaveClass('border-accent-edge', 'bg-accent-tint');
    expect(screen.getByRole('radio', { name: /timeline/i })).not.toHaveClass('bg-accent-tint');
  });

  it('reports the mode the user picked', async () => {
    const onSelect = vi.fn<(mode: 'graph' | 'timeline') => void>();
    render(<RunViewToggle mode="graph" onSelect={onSelect} />);

    await userEvent.click(screen.getByRole('radio', { name: /timeline/i }));

    expect(onSelect).toHaveBeenCalledExactlyOnceWith('timeline');
  });

});
