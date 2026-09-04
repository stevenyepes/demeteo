// The claim this file defends: `hidden` is additive-only and hides via class
// + `aria-hidden`, not unmounting — the graph's zoom state must survive a
// `'stacked'`-mode toggle (this ticket's own spec).

import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { indexTickets } from '../../lib/ticketPresentation';
import type { TicketView } from '../../types';
import { TicketColumn } from './TicketColumn';

afterEach(cleanup);

function ticket(id: string, seq: number): TicketView {
  return {
    ticket: {
      id,
      discovery_id: 'dsc-1',
      seq,
      title: `ticket ${seq}`,
      description: '',
      acceptance: [],
      files: [],
      blocked_by: [],
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
    standing: { id, lane: 'ready', startable: true, blockers: [] },
    feature: null,
  };
}

const TICKETS = [ticket('a', 1)];

function renderColumn(props: Partial<React.ComponentProps<typeof TicketColumn>> = {}) {
  return render(
    <TicketColumn
      tickets={TICKETS}
      index={indexTickets(TICKETS)}
      progress={null}
      selectedId={null}
      onSelect={() => {}}
      {...props}
    />,
  );
}

function rootDiv(container: HTMLElement): HTMLElement {
  return container.firstElementChild as HTMLElement;
}

describe('TicketColumn', () => {
  it('renders identically to today when hidden is omitted', () => {
    const { container } = renderColumn();

    const root = rootDiv(container);
    expect(root.classList.contains('hidden')).toBe(false);
    expect(root.getAttribute('aria-hidden')).toBeNull();
  });

  it('hides via class and aria-hidden without unmounting its content', () => {
    const { container } = renderColumn({ hidden: true });

    const root = rootDiv(container);
    expect(root.classList.contains('hidden')).toBe(true);
    expect(root.getAttribute('aria-hidden')).toBe('true');
    expect(screen.getByText('Graph')).toBeTruthy();
    expect(screen.getByText('Board')).toBeTruthy();
  });
});
