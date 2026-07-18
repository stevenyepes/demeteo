// Smoke tests for the ScrollArea primitive (TERMINALS_VIEW_SPEC §5, §9).
//
// These pin the load-bearing bits: the base scroll classes must stay on the
// root (they're what keep the area scrolling internally without shoving flex
// siblings off-screen), a caller-supplied className must survive the merge,
// and the ref must reach the underlying scrolling div so callers can drive
// scroll position imperatively.

import { createRef } from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { ScrollArea } from './ScrollArea';

describe('ScrollArea', () => {
  it('renders children', () => {
    render(<ScrollArea>hello world</ScrollArea>);

    expect(screen.getByText('hello world')).toBeInTheDocument();
  });

  it('applies the base scroll classes', () => {
    render(<ScrollArea />);

    const root = screen.getByTestId('scroll-area');
    expect(root).toHaveClass('overflow-y-auto');
    expect(root).toHaveClass('overscroll-contain');
    expect(root).toHaveClass('min-h-0');
  });

  it('merges a passed className after the base classes', () => {
    render(<ScrollArea className="border border-slate-700" />);

    const root = screen.getByTestId('scroll-area');
    expect(root).toHaveClass('overflow-y-auto');
    expect(root).toHaveClass('border');
    expect(root).toHaveClass('border-slate-700');
  });

  it('forwards a ref to the underlying div', () => {
    const ref = createRef<HTMLDivElement>();
    render(<ScrollArea ref={ref} />);

    expect(ref.current).toBeInstanceOf(HTMLDivElement);
    expect(ref.current).toBe(screen.getByTestId('scroll-area'));
  });
});
