import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import type { RunEvent } from '../lib/runs';
import { RunEvents } from './RunEvents';
import { TicketProgress } from './TicketProgress';

const NOW = 1_700_000_000;

function event(overrides: Partial<RunEvent>): RunEvent {
  return {
    offset: 0,
    kind: 'step_started',
    occurredAt: NOW - 60,
    stepId: null,
    stepExecutionId: null,
    ticketId: null,
    ...overrides,
  };
}

const EVENTS: RunEvent[] = [
  event({ offset: 1, kind: 'ticket_started', ticketId: 't-1', occurredAt: NOW - 600 }),
  event({ offset: 2, kind: 'ticket_started', ticketId: 't-2', occurredAt: NOW - 500 }),
  event({ offset: 3, kind: 'ticket_completed', ticketId: 't-1', occurredAt: NOW - 120 }),
  event({ offset: 4, kind: 'step_completed', stepId: 's-implement', occurredAt: NOW - 60 }),
];

function countOf(term: string): Element | null {
  return screen.getByText(term, { selector: 'dt' }).nextElementSibling;
}

describe('TicketProgress', () => {
  it('shows the completed, running and failed counts the mirror reported', () => {
    render(<TicketProgress detail={{ tickets: { completed: 4, failed: 2, running: 1 }, events: [] }} now={NOW} />);

    expect(countOf('Completed')).toHaveTextContent('4');
    expect(countOf('Running')).toHaveTextContent('1');
    expect(countOf('Running')).toHaveClass('text-cyan-300');
    expect(countOf('Failed')).toHaveTextContent('2');
    expect(countOf('Failed')).toHaveClass('text-ruby-300');
    expect(screen.getByText('No ticket events reported yet.')).toBeInTheDocument();
  });

  it('lists each ticket once, with the kind of its latest event', () => {
    render(<TicketProgress detail={{ tickets: { completed: 1, failed: 0, running: 1 }, events: EVENTS }} now={NOW} />);

    const rows = within(screen.getByRole('list', { name: 'Tickets' })).getAllByRole('listitem');
    expect(rows.map((row) => row.textContent)).toEqual(['t-1ticket_completed2m ago', 't-2ticket_started8m ago']);
  });
});

describe('RunEvents', () => {
  it('renders one row per event, newest first, with its offset, kind in words, ids and age', () => {
    render(<RunEvents events={EVENTS} now={NOW} />);

    const rows = within(screen.getByRole('list', { name: 'Events' })).getAllByRole('listitem');
    expect(rows.map((row) => row.textContent)).toEqual([
      '#4Step completed · s-implement1m ago',
      '#3Ticket completed · t-12m ago',
      '#2Ticket started · t-28m ago',
      '#1Ticket started · t-110m ago',
    ]);
  });

  it('shows the latest eight and offers the rest', async () => {
    const many = Array.from({ length: 12 }, (_, i) => event({ offset: i + 1, kind: 'gate_pending' }));
    render(<RunEvents events={many} now={NOW} />);

    const list = screen.getByRole('list', { name: 'Events' });
    expect(within(list).getAllByRole('listitem')).toHaveLength(8);
    expect(within(list).getAllByRole('listitem')[0]).toHaveTextContent(/^#12/);

    await userEvent.click(screen.getByRole('button', { name: 'Show all 12 events' }));

    expect(within(list).getAllByRole('listitem')).toHaveLength(12);
  });

  it('names a step by the title it was given, and by its id otherwise', () => {
    render(
      <RunEvents
        events={[event({ offset: 5, stepId: 's-plan' }), event({ offset: 6, stepId: 's-other' })]}
        now={NOW}
        stepTitles={{ 's-plan': 'Plan work' }}
      />,
    );

    const rows = within(screen.getByRole('list', { name: 'Events' })).getAllByRole('listitem');
    expect(rows.map((row) => row.textContent)).toEqual([
      '#6Step started · s-other1m ago',
      '#5Step started · Plan work1m ago',
    ]);
  });

  it('shows a kind it has never heard of verbatim', () => {
    render(<RunEvents events={[event({ offset: 9, kind: 'quantum_rebase_v7' })]} now={NOW} />);

    expect(screen.getByText('quantum_rebase_v7')).toBeInTheDocument();
  });

  it('says so when no events were reported', () => {
    render(<RunEvents events={[]} now={NOW} />);

    expect(screen.getByText('No events reported yet.')).toBeInTheDocument();
    expect(screen.queryByRole('list')).toBeNull();
  });
});
