// The claim under test is that the placeholder *reserves the list's box* —
// the reason §3.4 of the redesign plan replaces the spinner at all. Asserting
// a div count instead would pass just as happily on a zero-height placeholder,
// which is the failure mode: the list collapses, then shoves the page down
// when the rows land.

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { PipelineListSkeleton } from './PipelineListSkeleton';

describe('PipelineListSkeleton', () => {
  it('reserves a pipeline-row-sized box for every row it stands in for', () => {
    render(<PipelineListSkeleton rows={2} />);

    const blocks = screen.getAllByTestId('skeleton-block');
    expect(blocks).toHaveLength(2);
    for (const block of blocks) {
      expect(block).toHaveStyle({ height: '120px' });
    }
  });

  it('reserves a screenful when the caller does not say how many', () => {
    render(<PipelineListSkeleton />);

    expect(screen.getAllByTestId('skeleton-block')).toHaveLength(3);
  });

  it('never collapses to nothing', () => {
    render(<PipelineListSkeleton rows={0} />);

    expect(screen.getAllByTestId('skeleton-block')).toHaveLength(1);
  });

  it('announces the list once rather than once per row', () => {
    render(<PipelineListSkeleton rows={3} />);

    const status = screen.getByRole('status');
    expect(status).toHaveAccessibleName('Loading feature pipelines');
    expect(status).toHaveAttribute('aria-busy', 'true');
    for (const block of screen.getAllByTestId('skeleton-block')) {
      expect(block.closest('[aria-hidden="true"]')).not.toBeNull();
    }
  });

  it('shimmers through the class the reduced-motion block disables', () => {
    render(<PipelineListSkeleton rows={1} />);

    expect(screen.getByTestId('skeleton-block')).toHaveClass('animate-skeleton-pulse');
  });
});
