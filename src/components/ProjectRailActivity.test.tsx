import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import ProjectRailActivity from './ProjectRailActivity';

describe('ProjectRailActivity', () => {
  it('renders the attention badge when a run is waiting on a human', () => {
    render(<ProjectRailActivity activity={{ active: 0, needsYou: 1 }} />);

    const attention = screen.getByTestId('rail-project-attention');
    expect(attention).toHaveTextContent('1');
    expect(attention).toHaveAccessibleName('1 needs your attention');
    expect(attention.className).toContain('bg-amber-500/20');
    expect(attention.className).not.toContain('ruby');
  });

  it('renders the active count alone when nothing needs a human', () => {
    render(<ProjectRailActivity activity={{ active: 2, needsYou: 0 }} />);

    const running = screen.getByTestId('rail-project-active');
    expect(running).toHaveTextContent('2');
    expect(running).toHaveAccessibleName('2 active');
    expect(running.className).toContain('bg-cyan-500/20');
    expect(screen.queryByTestId('rail-project-attention')).toBeNull();
  });

  it('renders both badges when a project is running and gated at once', () => {
    render(<ProjectRailActivity activity={{ active: 3, needsYou: 1 }} />);

    expect(screen.getByTestId('rail-project-attention')).toHaveTextContent('1');
    expect(screen.getByTestId('rail-project-active')).toHaveTextContent('3');
  });

  it('renders nothing at all for a quiet project', () => {
    const { container } = render(<ProjectRailActivity activity={{ active: 0, needsYou: 0 }} />);

    expect(container.firstChild).toBeNull();
    expect(screen.queryByTestId('rail-project-attention')).toBeNull();
    expect(screen.queryByTestId('rail-project-active')).toBeNull();
  });

  it('keeps the attention badge and its name in the collapsed rail', () => {
    render(<ProjectRailActivity activity={{ active: 0, needsYou: 1 }} collapsed />);

    const attention = screen.getByTestId('rail-project-attention');
    expect(attention).toHaveTextContent('1');
    expect(attention).toHaveAccessibleName('1 needs your attention');
    expect(attention.className).toContain('-top-1');
    expect(attention.className).toContain('-left-1');
  });

  it('renders nothing at all for a quiet project when collapsed', () => {
    const { container } = render(
      <ProjectRailActivity activity={{ active: 0, needsYou: 0 }} collapsed />,
    );

    expect(container.firstChild).toBeNull();
  });

  // Pinned by attribute, not `toHaveAccessibleName` — see `RailBadge`.
  it('names the badge through a role that permits an author-supplied name', () => {
    render(<ProjectRailActivity activity={{ active: 2, needsYou: 1 }} />);

    for (const testId of ['rail-project-attention', 'rail-project-active']) {
      const badge = screen.getByTestId(testId);
      expect(badge).toHaveAttribute('role', 'img');
      expect(badge).toHaveAttribute('aria-label');
    }
  });

  it('clamps a count that would overflow the collapsed corner, keeping the real number in its name', () => {
    render(<ProjectRailActivity activity={{ active: 10, needsYou: 120 }} collapsed />);

    const attention = screen.getByTestId('rail-project-attention');
    expect(attention).toHaveTextContent('9+');
    expect(attention).toHaveAccessibleName('120 needs your attention');
    expect(screen.getByTestId('rail-project-active')).toHaveTextContent('9+');
  });

  it('lets the expanded pill count to 99 before it clamps', () => {
    render(<ProjectRailActivity activity={{ active: 42, needsYou: 120 }} />);

    expect(screen.getByTestId('rail-project-active')).toHaveTextContent('42');
    expect(screen.getByTestId('rail-project-attention')).toHaveTextContent('99+');
  });
});
