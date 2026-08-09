// The Graph|Timeline toggle, now a thin binding of `SegmentedControl` to the
// run column's view mode.
//
// Two things have to survive that binding: the selected mode still reads as
// selected through the shared `TONE_CHIP` treatment (a local cyan style here is
// the F27 drift the redesign closed), and `onSelect` still speaks `RunViewMode`
// rather than the primitive's generic.
//
// Measurement is no longer this component's: the run view seats it in a chrome
// row beside the density toggle and measures that row, and
// `FeatureDetail.layout.test.tsx` carries the claim that used to live here.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { TONE_CHIP } from '../lib/runStatus';
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

  it('dresses the active mode in the shared cyan chip, not a local one', () => {
    render(<RunViewToggle mode="graph" onSelect={() => {}} />);

    expect(screen.getByRole('radio', { name: /graph/i }).className).toContain(TONE_CHIP.cyan);
    expect(screen.getByRole('radio', { name: /timeline/i }).className).not.toContain(TONE_CHIP.cyan);
  });

  it('reports the mode the user picked', async () => {
    const onSelect = vi.fn<(mode: 'graph' | 'timeline') => void>();
    render(<RunViewToggle mode="graph" onSelect={onSelect} />);

    await userEvent.click(screen.getByRole('radio', { name: /timeline/i }));

    expect(onSelect).toHaveBeenCalledExactlyOnceWith('timeline');
  });

});
