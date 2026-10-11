import { render, screen } from '@testing-library/react';
import type { ReactElement } from 'react';
import { describe, expect, it } from 'vitest';

import { AddInstance } from './AddInstance';
import { Dispatch } from './Dispatch';
import { Fleet } from './Fleet';
import { Instance } from './Instance';
import { NotFound } from './NotFound';
import { Run } from './Run';
import { Runs } from './Runs';

const VIEWS: { heading: string; element: ReactElement }[] = [
  { heading: 'Fleet', element: <Fleet /> },
  { heading: 'Runs', element: <Runs /> },
  { heading: 'Dispatch', element: <Dispatch /> },
  { heading: 'Add instance', element: <AddInstance /> },
  { heading: 'Instance', element: <Instance instanceId="inst-7" /> },
  { heading: 'Run', element: <Run instanceId="inst-7" featureId="feat-42" /> },
  { heading: 'Not found', element: <NotFound path="/nowhere" /> },
];

function monoTexts(container: HTMLElement): (string | null)[] {
  return Array.from(container.querySelectorAll('.font-mono'), (element) => element.textContent);
}

describe('placeholder views', () => {
  it.each(VIEWS)('$heading renders its heading in the glass card', ({ heading, element }) => {
    render(element);

    const headings = screen.getAllByRole('heading', { level: 1 });
    expect(headings).toHaveLength(1);
    expect(headings[0]).toHaveTextContent(new RegExp(`^${heading}$`));
    expect(headings[0]).toHaveClass('font-heading');
    expect(headings[0].closest('.glass-panel')).not.toBeNull();
  });

  it('Instance shows its instance id in monospace', () => {
    const { container } = render(<Instance instanceId="inst-7" />);

    expect(monoTexts(container)).toEqual(['inst-7']);
  });

  it('Run shows the instance id, then the feature id, in monospace', () => {
    const { container } = render(<Run instanceId="inst-7" featureId="feat-42" />);

    expect(monoTexts(container)).toEqual(['inst-7', 'feat-42']);
  });

  it('NotFound shows the unmatched path and links back to Fleet', () => {
    const { container } = render(<NotFound path="/nowhere" />);

    expect(monoTexts(container)).toEqual(['/nowhere']);
    expect(screen.getByRole('link')).toHaveAttribute('href', '#/fleet');
  });

  it('renders an id that looks like markup as text', () => {
    const { container } = render(<Instance instanceId="<b>x</b>" />);

    expect(monoTexts(container)).toEqual(['<b>x</b>']);
    expect(container.querySelector('b')).toBeNull();
  });
});
