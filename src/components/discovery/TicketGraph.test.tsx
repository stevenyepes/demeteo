/**
 * The zoom arithmetic is `lib/canvasZoom.test.ts`; what needs a DOM is which
 * gesture reaches it and which one it must not undo. A panel resize should
 * reach a fit with no click — the way `useDiscoveryColumnLayout` reaches its
 * layout decision from a triggered `ResizeObserverStub` tick rather than
 * `entry.contentRect`, which jsdom never fills in — and it must stop doing
 * that once the operator has zoomed themselves.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { indexTickets } from '../../lib/ticketPresentation';
import { resizeObserverStubs } from '../../test/setup';
import type { TicketView } from '../../types';
import { TicketGraph } from './TicketGraph';

function ticket(id: string, seq: number, blockedBy: string[]): TicketView {
  return {
    ticket: {
      id,
      discovery_id: 'dsc-1',
      seq,
      title: `ticket ${seq}`,
      description: '',
      acceptance: [],
      files: [],
      blocked_by: blockedBy,
      test_command: null,
      workflow_id: null,
      agent_kind: null,
      model: null,
      effort: null,
      attachments: [],
      state: 'unstarted',
      drop_reason: null,
      force_start_reason: null,
      force_started_at: null,
      feature_id: null,
      created_at: 0,
      updated_at: 0,
    },
    standing: {
      id,
      lane: 'blocked',
      startable: false,
      blockers: blockedBy.map((blocker) => ({ id: blocker, reason: 'outstanding' })),
    },
    feature: null,
  };
}

const TICKETS = [ticket('a', 1, []), ticket('b', 2, ['a'])];

function observerFor(target: Element) {
  const observer = resizeObserverStubs.find((o) => o.observe.mock.calls.some(([t]) => t === target));
  if (!observer) throw new Error('no ResizeObserver was registered for the viewport');
  return observer;
}

function scaleOf(container: HTMLElement): string {
  const scaled = container.querySelector('.origin-top-left') as HTMLElement | null;
  if (!scaled) throw new Error('no scaled node rendered');
  return scaled.style.transform;
}

describe('TicketGraph centring', () => {
  // jsdom lays nothing out, so what is assertable is the contract the three
  // classes make together — and each of the three is individually removable
  // without any other test noticing: drop `flex` or `m-auto` and the graph
  // sits against the left edge of a pane wider than it, drop `shrink-0` and
  // the canvas is squeezed to the pane instead of scrolling.
  it('centres the canvas in the leftover pane without stranding the overflow', () => {
    render(
      <TicketGraph
        tickets={TICKETS}
        index={indexTickets(TICKETS)}
        selectedId={null}
        onSelect={() => {}}
      />,
    );

    const viewport = screen.getByTestId('ticket-graph').firstElementChild as HTMLElement;
    const canvas = screen.getByTestId('ticket-graph-canvas');

    expect(viewport).toHaveClass('flex', 'overflow-auto');
    expect(canvas).toHaveClass('m-auto', 'shrink-0');
    // Separately: `.not.toHaveClass(a, b)` passes when only one is absent,
    // which is how a container-centred scroller slips past this assertion.
    expect(viewport).not.toHaveClass('justify-center');
    expect(viewport).not.toHaveClass('items-center');
    expect(canvas.parentElement).toBe(viewport);
  });
});

describe('TicketGraph auto-fit on resize', () => {
  it('re-fits from a measured viewport resize with no button click', () => {
    const { container } = render(
      <TicketGraph
        tickets={TICKETS}
        index={indexTickets(TICKETS)}
        selectedId={null}
        onSelect={() => {}}
      />,
    );

    const viewport = screen.getByTestId('ticket-graph').firstElementChild as HTMLElement;
    expect(scaleOf(container)).toBe('scale(1)');

    Object.defineProperty(viewport, 'clientWidth', { configurable: true, value: 400 });
    Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 300 });

    act(() => observerFor(viewport).trigger());

    expect(scaleOf(container)).not.toBe('scale(1)');
  });
});

describe('TicketGraph gestures', () => {
  function mount() {
    const view = render(
      <TicketGraph
        tickets={TICKETS}
        index={indexTickets(TICKETS)}
        selectedId={null}
        onSelect={() => {}}
      />,
    );
    const pane = screen.getByTestId('ticket-graph').firstElementChild as HTMLElement;
    Object.defineProperty(pane, 'clientWidth', { configurable: true, value: 400 });
    Object.defineProperty(pane, 'clientHeight', { configurable: true, value: 300 });
    return { ...view, pane };
  }

  /**
   * The bug the auto-fit guard exists for, and the reason the +/− buttons read
   * as dead: zooming past the pane raises the scrollbars, a scrollbar shrinks
   * the pane's content box, the observer ticks, and an unguarded refit puts the
   * zoom back within the frame. Nothing in the DOM says a button was pressed —
   * it simply does not work.
   */
  it('does not undo the operator zoom on the next pane resize', () => {
    const { container, pane } = mount();

    act(() => observerFor(pane).trigger());
    const fitted = scaleOf(container);

    fireEvent.click(screen.getByLabelText('Zoom in'));
    const zoomed = scaleOf(container);
    expect(zoomed).not.toBe(fitted);

    Object.defineProperty(pane, 'clientWidth', { configurable: true, value: 385 });
    act(() => observerFor(pane).trigger());

    expect(scaleOf(container)).toBe(zoomed);
  });

  it('re-arms the framing when Fit is pressed', () => {
    const { container, pane } = mount();

    fireEvent.click(screen.getByLabelText('Zoom in'));
    fireEvent.click(screen.getByLabelText('Fit to view'));
    const fitted = scaleOf(container);

    act(() => observerFor(pane).trigger());
    expect(scaleOf(container)).toBe(fitted);
  });

  it('zooms on the wheel', () => {
    const { container, pane } = mount();
    const before = scaleOf(container);

    fireEvent.wheel(pane, { deltaY: -240, clientX: 200, clientY: 150 });

    expect(scaleOf(container)).not.toBe(before);
    expect(screen.getByLabelText('Reset zoom to 100%')).not.toHaveTextContent('100%');
  });

  // A card is a button, so a drag that starts on one is someone missing a
  // click, not someone panning — pan it and the click never lands.
  it('pans from the background and not from a card', () => {
    const { pane } = mount();

    fireEvent.pointerDown(pane, { pointerId: 1, button: 0, clientX: 10, clientY: 10 });
    expect(pane).toHaveClass('cursor-grabbing');

    fireEvent.pointerUp(pane, { pointerId: 1 });
    expect(pane).toHaveClass('cursor-grab');

    fireEvent.pointerDown(screen.getAllByTestId('ticket-node')[0], {
      pointerId: 2,
      button: 0,
      clientX: 10,
      clientY: 10,
    });
    expect(pane).toHaveClass('cursor-grab');
  });
});
