/**
 * The manual 'Fit' button already covers the zoom math (`ticketGraphLayout.test.ts`
 * pins the layout it divides into). What's untested is the automatic half: a
 * panel resize should reach the same `fit()` with no click, the way
 * `useDiscoveryColumnLayout` reaches its layout decision from a triggered
 * `ResizeObserverStub` tick rather than `entry.contentRect`, which jsdom never
 * fills in.
 */
import { act, render, screen } from '@testing-library/react';
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
