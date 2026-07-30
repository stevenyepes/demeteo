// The Graph|Timeline toggle, extracted out of `FeatureDetail` when the run
// layout moved into `useRunColumnLayout`.
//
// Two things have to survive the move: the selected tab still reads as selected
// (the cyan treatment is the only affordance saying which view you're in), and
// the element still hands its node to the `chromeRef` it was given — that ref is
// how the layout hook measures the chrome above the graph, so a dropped ref
// silently over-states the space the graph box has.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { RunViewToggle } from './RunViewToggle';

describe('RunViewToggle', () => {
  it('marks the active mode and reports the other on click', async () => {
    const onSelect = vi.fn();
    render(<RunViewToggle mode="graph" onSelect={onSelect} chromeRef={() => {}} />);

    expect(screen.getByRole('button', { name: /graph/i }).className).toContain('text-cyan-300');
    expect(screen.getByRole('button', { name: /timeline/i }).className).not.toContain('text-cyan-300');

    await userEvent.click(screen.getByRole('button', { name: /timeline/i }));
    expect(onSelect).toHaveBeenCalledWith('timeline');
  });

  it('hands its own element to the layout hook s chrome ref', () => {
    const chromeRef = vi.fn();
    render(<RunViewToggle mode="timeline" onSelect={() => {}} chromeRef={chromeRef} />);

    expect(chromeRef).toHaveBeenCalledTimes(1);
    expect(chromeRef.mock.calls[0][0]).toBeInstanceOf(HTMLDivElement);
    expect(chromeRef.mock.calls[0][0].textContent).toContain('Graph');
  });
});
