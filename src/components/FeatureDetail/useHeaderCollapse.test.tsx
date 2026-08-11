/**
 * The header collapses for the run surface's scroll and for nothing else.
 *
 * Both claims here are invisible to the pure `headerCollapse` module, which is
 * handed an offset and never asked where it came from: a listener without the
 * capture phase sees no scroll at all in the side layout, and one without the
 * attribute filter sees every scroll in the column — the inspector's tab bodies
 * included. Neither shows up as an error anywhere.
 */
import { act, render } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it } from 'vitest';

import { useHeaderCollapse } from './useHeaderCollapse';

/** The column mounts through state exactly as `useRunColumnLayout` reports it,
 *  so the hook is exercised across the `null` render every real mount begins
 *  with rather than being handed a live element it never sees first. */
function Harness({ gateStepExecutionId = null }: { gateStepExecutionId?: string | null }) {
  const [el, setEl] = useState<HTMLDivElement | null>(null);
  const collapsed = useHeaderCollapse(el, gateStepExecutionId);
  return (
    <div ref={setEl} data-testid="column">
      <span data-testid="state">{collapsed ? 'collapsed' : 'full'}</span>
      <div data-testid="surface" data-run-scroll />
      <div data-testid="inspector-body" />
    </div>
  );
}

/** The hook coalesces to one animation frame, so the assertion has to wait for
 *  the frame rather than for the event. */
async function scroll(el: HTMLElement, scrollTop: number) {
  await act(async () => {
    Object.defineProperty(el, 'scrollTop', { value: scrollTop, configurable: true });
    el.dispatchEvent(new Event('scroll'));
    await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
  });
}

describe('useHeaderCollapse', () => {
  it('collapses for a scroll in a descendant surface', async () => {
    const { getByTestId } = render(<Harness />);
    expect(getByTestId('state')).toHaveTextContent('full');

    await scroll(getByTestId('surface'), 400);

    expect(getByTestId('state')).toHaveTextContent('collapsed');
  });

  it('expands again once that surface is back at the top', async () => {
    const { getByTestId } = render(<Harness />);
    await scroll(getByTestId('surface'), 400);
    expect(getByTestId('state')).toHaveTextContent('collapsed');

    await scroll(getByTestId('surface'), 60);
    expect(getByTestId('state')).toHaveTextContent('collapsed');

    await scroll(getByTestId('surface'), 0);
    expect(getByTestId('state')).toHaveTextContent('full');
  });

  it('ignores a scroll in anything that is not the run surface', async () => {
    const { getByTestId } = render(<Harness />);

    await scroll(getByTestId('inspector-body'), 400);

    expect(getByTestId('state')).toHaveTextContent('full');
  });

  it('re-establishes the header when a gate overlay closes', async () => {
    const { getByTestId, rerender } = render(<Harness gateStepExecutionId="se-f1-s-gate" />);
    // What `useGateCardScroll` does under the overlay, which no user asked for.
    await scroll(getByTestId('surface'), 400);
    expect(getByTestId('state')).toHaveTextContent('collapsed');

    rerender(<Harness gateStepExecutionId={null} />);

    expect(getByTestId('state')).toHaveTextContent('full');
  });

  it('collapses for the column when the column is itself the surface', async () => {
    const { getByTestId } = render(<Harness />);
    const column = getByTestId('column');
    column.setAttribute('data-run-scroll', '');

    await scroll(column, 400);

    expect(getByTestId('state')).toHaveTextContent('collapsed');
  });
});
