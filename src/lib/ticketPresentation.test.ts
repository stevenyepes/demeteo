// Two things here are corrections to the mocks rather than transcriptions of
// them, and both are recorded in `docs/TASKS_DISCOVERY.md`: a dropped lane that
// still renders all five even when empty, and a closed-unmerged ticket that
// shares that lane without borrowing its reason.

import { describe, expect, it } from 'vitest';

import {
  TICKET_LANES,
  bucketByLane,
  dropNote,
  indexTickets,
  primaryAction,
  stateLabel,
  ticketTone,
} from './ticketPresentation';
import type { TicketLane, TicketState, TicketView } from '../types';

function ticket(
  id: string,
  seq: number,
  lane: TicketLane,
  overrides: {
    state?: TicketState;
    blockedBy?: string[];
    blockers?: string[];
    dropReason?: string | null;
    mrUrl?: string | null;
  } = {},
): TicketView {
  const blockedBy = overrides.blockedBy ?? [];
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
      state: overrides.state ?? 'unstarted',
      drop_reason: overrides.dropReason ?? null,
      force_start_reason: null,
      force_started_at: null,
      feature_id: null,
      created_at: 0,
      updated_at: 0,
    },
    standing: {
      id,
      lane,
      startable: lane === 'ready',
      blockers: (overrides.blockers ?? blockedBy).map((blocker) => ({
        id: blocker,
        reason: 'outstanding',
      })),
    },
    feature: overrides.mrUrl
      ? { id: `f-${id}`, status: 'completed', mr_state: 'closed', mr_url: overrides.mrUrl }
      : null,
  };
}

describe('bucketByLane', () => {
  it('renders all five lanes, in order, with nothing in them', () => {
    expect(bucketByLane([]).map((bucket) => bucket.meta.lane)).toEqual(
      TICKET_LANES.map((lane) => lane.lane),
    );
    expect(bucketByLane([]).every((bucket) => bucket.tickets.length === 0)).toBe(true);
  });

  it('puts each ticket in the lane the board derived', () => {
    const tickets = [ticket('a', 1, 'landed'), ticket('b', 2, 'ready'), ticket('c', 3, 'ready')];
    const byLane = new Map(bucketByLane(tickets).map((b) => [b.meta.lane, b.tickets.length]));

    expect(byLane.get('ready')).toBe(2);
    expect(byLane.get('landed')).toBe(1);
    expect(byLane.get('blocked')).toBe(0);
  });
});

describe('ticketTone', () => {
  it('grades blocked amber once a prerequisite is in flight', () => {
    const tickets = [ticket('a', 1, 'in_flight'), ticket('b', 2, 'blocked', { blockedBy: ['a'] })];
    const index = indexTickets(tickets);

    expect(ticketTone(tickets[1], index)).toBe('amber');
  });

  it('leaves blocked slate while nothing has started', () => {
    const tickets = [ticket('a', 1, 'blocked'), ticket('b', 2, 'blocked', { blockedBy: ['a'] })];
    const index = indexTickets(tickets);

    expect(ticketTone(tickets[1], index)).toBe('slate');
  });
});

describe('a closed-unmerged ticket', () => {
  const closed = ticket('a', 1, 'dropped', {
    state: 'started',
    mrUrl: 'https://github.com/o/r/pull/134',
  });

  it('says its PR closed rather than borrowing the lane note', () => {
    expect(stateLabel(closed)).toBe('Closed');
    expect(dropNote(closed)).toBe('PR #134 closed without merging');
  });

  it('never renders an absent drop reason as though it had one', () => {
    const dropped = ticket('b', 2, 'dropped', { state: 'dropped', dropReason: null });

    expect(dropNote(dropped)).toBeNull();
    expect(dropNote(ticket('c', 3, 'dropped', { state: 'dropped', dropReason: 'folded in' }))).toBe(
      'folded in',
    );
  });
});

describe('primaryAction', () => {
  it('names the one blocker, and counts the rest', () => {
    const tickets = [
      ticket('a', 1, 'blocked'),
      ticket('b', 2, 'blocked'),
      ticket('c', 3, 'blocked', { blockedBy: ['a'] }),
      ticket('d', 4, 'blocked', { blockedBy: ['a', 'b'] }),
    ];
    const index = indexTickets(tickets);

    expect(primaryAction(tickets[2], index)).toMatchObject({
      label: 'Blocked by DSC-1',
      disabled: true,
    });
    expect(primaryAction(tickets[3], index)).toMatchObject({
      label: 'Blocked by 2 tickets',
      disabled: true,
    });
  });

  it('offers to start a ready ticket and to open a started one', () => {
    const ready = ticket('a', 1, 'ready');
    const started = ticket('b', 2, 'in_flight', {
      state: 'started',
      mrUrl: 'https://github.com/o/r/pull/9',
    });
    const index = indexTickets([ready, started]);

    expect(primaryAction(ready, index)).toMatchObject({ label: 'Start ticket', kind: 'start' });
    expect(primaryAction(started, index)).toMatchObject({ label: 'Open feature', kind: 'open' });
  });
});
