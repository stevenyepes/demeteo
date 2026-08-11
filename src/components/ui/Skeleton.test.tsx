/// <reference types="node" />

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Skeleton } from './Skeleton';

// The stylesheet is read from disk rather than imported: vitest runs with
// `css: false`, which stubs every CSS module to an empty string — `?raw`
// included — so an import would assert against nothing. `node:fs` needs the
// triple-slash reference above because the project tsconfig declares no `types`.
const APP_CSS = readFileSync(resolve(process.cwd(), 'src/App.css'), 'utf8');

function reducedMotionBlock(css: string): string {
  const start = css.indexOf('@media (prefers-reduced-motion: reduce)');
  expect(start).toBeGreaterThan(-1);
  const end = css.indexOf('\n}', start);
  expect(end).toBeGreaterThan(start);
  return css.slice(start, end);
}

describe('Skeleton', () => {
  it('announces that content is loading instead of exposing the boxes', () => {
    render(<Skeleton count={3} />);

    const status = screen.getByRole('status');
    expect(status).toHaveAccessibleName('Loading');
    expect(status).toHaveAttribute('aria-busy', 'true');

    for (const bar of status.querySelectorAll('.animate-skeleton-pulse')) {
      expect(bar.closest('[aria-hidden="true"]')).not.toBeNull();
    }
  });

  it('takes a caller label so the announcement names the content', () => {
    render(<Skeleton label="Loading feature pipelines" />);

    expect(screen.getByRole('status')).toHaveAccessibleName('Loading feature pipelines');
  });

  it('renders one bar per line and shortens the last of a paragraph', () => {
    render(<Skeleton count={3} />);

    const bars = screen.getAllByTestId('skeleton-line');
    expect(bars).toHaveLength(3);
    expect(bars[0]).toHaveClass('w-full');
    expect(bars[2]).toHaveClass('w-3/4');
  });

  it('keeps a single line full width', () => {
    render(<Skeleton />);

    const bars = screen.getAllByTestId('skeleton-line');
    expect(bars).toHaveLength(1);
    expect(bars[0]).toHaveClass('w-full');
  });

  it('holds the box a block placeholder stands in for', () => {
    render(<Skeleton variant="block" height={320} />);

    expect(screen.getByTestId('skeleton-block')).toHaveStyle({ height: '320px' });
  });

  it('accepts a CSS length for the block height', () => {
    render(<Skeleton variant="block" height="100%" />);

    expect(screen.getByTestId('skeleton-block')).toHaveStyle({ height: '100%' });
  });

  it('renders card placeholders under one live region, not one each', () => {
    render(<Skeleton variant="card" count={3} />);

    expect(screen.getAllByTestId('skeleton-card')).toHaveLength(3);
    expect(screen.getAllByRole('status')).toHaveLength(1);
  });

  it('stays out of the accessibility tree when the caller already announces', () => {
    const { container } = render(<Skeleton announce={false} count={2} />);

    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    expect(container.firstElementChild).toHaveAttribute('aria-hidden', 'true');
    expect(screen.getAllByTestId('skeleton-line')).toHaveLength(2);
  });

  it('shimmers through the shared opacity-only class in every variant', () => {
    const { container } = render(
      <>
        <Skeleton count={2} />
        <Skeleton variant="block" height={40} />
        <Skeleton variant="card" />
      </>,
    );

    expect(container.querySelectorAll('.animate-skeleton-pulse').length).toBeGreaterThan(3);
  });

  it('defines the shimmer as an opacity-only keyframe in App.css', () => {
    expect(APP_CSS).toContain('@keyframes skeleton-pulse');
    expect(APP_CSS).toContain('.animate-skeleton-pulse');

    const keyframe = APP_CSS.slice(
      APP_CSS.indexOf('@keyframes skeleton-pulse'),
      APP_CSS.indexOf('.animate-skeleton-pulse'),
    );
    expect(keyframe).toContain('opacity');
    expect(keyframe).not.toMatch(/transform|box-shadow|scale/);
  });

  it('is disabled under prefers-reduced-motion', () => {
    expect(reducedMotionBlock(APP_CSS)).toContain('.animate-skeleton-pulse');
  });
});
