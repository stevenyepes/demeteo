// Behaviour tests for the density toggle (UI_REDESIGN_PLAN §3.7).
//
// It owns almost nothing — the point of these is that it stays that way: it is
// the shared SegmentedControl (§5.1 forbids a second one), it reports the
// typed density rather than a string, and it holds no state of its own, so
// Phase 6 can persist the value without unpicking a local copy.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { DensityToggle } from './DensityToggle';

describe('DensityToggle', () => {
  it('is the shared segmented control, named for what it changes', () => {
    render(<DensityToggle value="comfortable" onChange={() => {}} />);

    expect(screen.getByTestId('segmented-control')).toBe(
      screen.getByRole('radiogroup', { name: 'Timeline density' }),
    );
  });

  it('offers exactly the two densities', () => {
    render(<DensityToggle value="comfortable" onChange={() => {}} />);

    expect(screen.getAllByRole('radio').map((el) => el.textContent)).toEqual([
      'Comfortable',
      'Compact',
    ]);
  });

  it('checks the density it was given', () => {
    render(<DensityToggle value="compact" onChange={() => {}} />);

    expect(screen.getByRole('radio', { name: 'Compact' })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('radio', { name: 'Comfortable' })).toHaveAttribute(
      'aria-checked',
      'false',
    );
  });

  it('reports the picked density to the caller', async () => {
    const onChange = vi.fn();
    render(<DensityToggle value="comfortable" onChange={onChange} />);

    await userEvent.click(screen.getByRole('radio', { name: 'Compact' }));

    expect(onChange).toHaveBeenCalledWith('compact');
  });

  it('keeps no state of its own — the checked segment follows the prop', async () => {
    const { rerender } = render(<DensityToggle value="comfortable" onChange={() => {}} />);

    await userEvent.click(screen.getByRole('radio', { name: 'Compact' }));
    expect(screen.getByRole('radio', { name: 'Compact' })).toHaveAttribute('aria-checked', 'false');

    rerender(<DensityToggle value="compact" onChange={() => {}} />);
    expect(screen.getByRole('radio', { name: 'Compact' })).toHaveAttribute('aria-checked', 'true');
  });

  it('renders at the denser size, so it fits the run toolbar', () => {
    render(<DensityToggle value="comfortable" onChange={() => {}} className="ml-auto" />);

    const group = screen.getByTestId('segmented-control');
    expect(group).toHaveAttribute('data-size', 'sm');
    expect(group).toHaveClass('ml-auto');
  });
});
