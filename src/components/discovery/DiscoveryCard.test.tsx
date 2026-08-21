// The Project Home card must read the same counter the workspace does
// (`docs/PRD_DISCOVERY.md` §9.2). `DISCOVERY_UI_SPEC.md` §3.5.1 records that
// the mocks do not: the same seven tickets, one dropped, are drawn as `1 of 7`
// here and `1 of 6` there. This pins the card to the derived value.

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { DiscoveryCard } from './DiscoveryCard';
import type {
  DiscoveryBoard,
  DiscoverySummary,
  TicketLane,
  TicketProgress,
  TicketView,
} from '../../types';

afterEach(cleanup);

const NOTHING_PROPOSED: TicketProgress = {
  blocked: 0,
  ready: 0,
  in_flight: 0,
  landed: 0,
  dropped: 0,
  live: 0,
};

function discovery(overrides: Partial<DiscoverySummary> = {}): DiscoverySummary {
  return {
    id: 'dsc-1',
    project_id: 'p1',
    title: 'Runner serves more than one client',
    status: 'open',
    machine_id: 'local',
    agent_kind: 'claude-code',
    model: 'opus',
    effort: 'high',
    resume_session_id: null,
    worktree_path: null,
    attachments: [],
    total_cost: 2.14,
    tokens: 486_000,
    created_at: 0,
    updated_at: 0,
    message_count: 4,
    progress: NOTHING_PROPOSED,
    ...overrides,
  };
}

function ticket(seq: number, lane: TicketLane): TicketView {
  return {
    ticket: {
      id: `t${seq}`,
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
      state: lane === 'dropped' ? 'dropped' : 'unstarted',
      drop_reason: null,
      force_start_reason: null,
      force_started_at: null,
      feature_id: null,
      created_at: 0,
      updated_at: 0,
    },
    standing: { id: `t${seq}`, lane, startable: false, blockers: [] },
    feature: null,
  };
}

const SEVEN_ONE_DROPPED: DiscoveryBoard = {
  tickets: [
    ticket(1, 'landed'),
    ticket(2, 'in_flight'),
    ticket(3, 'blocked'),
    ticket(4, 'ready'),
    ticket(5, 'blocked'),
    ticket(6, 'dropped'),
    ticket(7, 'blocked'),
  ],
  progress: { blocked: 3, ready: 1, in_flight: 1, landed: 1, dropped: 1, live: 6 },
};

describe('DiscoveryCard', () => {
  it('counts landed against live tickets, not against the whole set', () => {
    render(
      <DiscoveryCard
        discovery={discovery({ progress: SEVEN_ONE_DROPPED.progress })}
        board={SEVEN_ONE_DROPPED}
        turnRunning={false}
        now={0}
        onOpen={() => {}}
      />,
    );

    expect(screen.getByText('1 of 6 landed · 1 in flight')).toBeTruthy();
    expect(screen.queryByText(/of 7 landed/)).toBeNull();
    expect(screen.getByTestId('ticket-progress-bar').getAttribute('title')).toBe(
      '1 of 6 landed · 1 in flight',
    );
  });

  // The counter rides on the list row, so it is drawn with the card rather
  // than a board fetch later — which is what `DiscoverySummary` exists for.
  it('draws the bar from the list row, before any board has answered', () => {
    render(
      <DiscoveryCard
        discovery={discovery({ progress: SEVEN_ONE_DROPPED.progress })}
        board={null}
        turnRunning={false}
        now={0}
        onOpen={() => {}}
      />,
    );

    expect(screen.getByText('1 of 6 landed · 1 in flight')).toBeTruthy();
  });

  it('reads its turn count off the row', () => {
    render(
      <DiscoveryCard
        discovery={discovery({ message_count: 4 })}
        board={null}
        turnRunning={false}
        now={0}
        onOpen={() => {}}
      />,
    );

    expect(screen.getByText('4 turns')).toBeTruthy();
  });

  it('says one turn, not one turns', () => {
    render(
      <DiscoveryCard
        discovery={discovery({ message_count: 1 })}
        board={null}
        turnRunning={false}
        now={0}
        onOpen={() => {}}
      />,
    );

    expect(screen.getByText('1 turn')).toBeTruthy();
  });

  it('shows no progress arithmetic before anything has been proposed', () => {
    render(
      <DiscoveryCard
        discovery={discovery()}
        board={{ tickets: [], progress: NOTHING_PROPOSED }}
        turnRunning={false}
        now={0}
        onOpen={() => {}}
      />,
    );

    expect(screen.queryByTestId('ticket-progress-bar')).toBeNull();
  });

  it('opens the discovery it was clicked on', () => {
    const onOpen = vi.fn();
    render(
      <DiscoveryCard
        discovery={discovery()}
        board={null}
        turnRunning={false}
        now={0}
        onOpen={onOpen}
      />,
    );

    fireEvent.click(screen.getByTestId('discovery-card'));

    expect(onOpen).toHaveBeenCalledWith('dsc-1', 'Runner serves more than one client');
  });
});
