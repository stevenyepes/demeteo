import { describe, expect, it } from 'vitest';

import {
  discoveryDetailLine,
  discoveryLifecycle,
  prNumber,
  progressSegments,
  progressText,
  ticketLabel,
  turnCountLabel,
} from './discoveryProgress';
import type {
  Discovery,
  DiscoveryBoard,
  TicketFeatureView,
  TicketLane,
  TicketView,
} from '../types';

function ticket(
  seq: number,
  lane: TicketLane,
  extra: {
    startable?: boolean;
    blockedBy?: string[];
    /** The *unsatisfied* subset, which is what `TicketStanding.blockers`
     *  carries — a prerequisite that merged is an edge, not a blocker. */
    unmet?: string[];
    feature?: TicketFeatureView | null;
  } = {},
): TicketView {
  const id = `t${seq}`;
  return {
    ticket: {
      id,
      discovery_id: 'dsc-1',
      seq,
      title: `ticket ${seq}`,
      description: '',
      acceptance: [],
      files: [],
      blocked_by: extra.blockedBy ?? [],
      test_command: null,
      workflow_id: null,
      agent_kind: null,
      model: null,
      effort: null,
      attachments: [],
      state: lane === 'dropped' ? 'dropped' : lane === 'blocked' || lane === 'ready' ? 'unstarted' : 'started',
      drop_reason: null,
      force_start_reason: null,
      force_started_at: null,
      feature_id: extra.feature?.id ?? null,
      created_at: 0,
      updated_at: 0,
    },
    standing: {
      id,
      lane,
      startable: extra.startable ?? lane === 'ready',
      blockers: (extra.unmet ?? []).map((b) => ({ id: b, reason: 'outstanding' as const })),
    },
    feature: extra.feature ?? null,
  };
}

/** The seven tickets of `DISCOVERY_UI_SPEC.md` §3.5.5 — one landed, one
 *  running, three blocked, one ready, one dropped. */
function seededBoard(): DiscoveryBoard {
  const tickets = [
    ticket(1, 'landed', {
      feature: { id: 'f1', status: 'completed', mr_state: 'merged', mr_url: 'https://x/pull/131' },
    }),
    ticket(2, 'in_flight', {
      feature: { id: 'f2', status: 'running', mr_state: 'open', mr_url: 'https://x/pull/134' },
    }),
    ticket(3, 'blocked', { blockedBy: ['t1', 't2'], unmet: ['t2'] }),
    ticket(4, 'ready', { blockedBy: ['t1'] }),
    ticket(5, 'blocked', { blockedBy: ['t3'], unmet: ['t3'] }),
    ticket(6, 'dropped'),
    ticket(7, 'blocked', { blockedBy: ['t3', 't5'], unmet: ['t3', 't5'] }),
  ];
  return {
    tickets,
    progress: { blocked: 3, ready: 1, in_flight: 1, landed: 1, dropped: 1, live: 6 },
  };
}

describe('progressText', () => {
  // The trap `DISCOVERY_UI_SPEC.md` §3.5.1 records: the mocks show `1 of 7` on
  // Project Home's card and `1 of 6` in the workspace for the same seven
  // tickets, one of them dropped. §9.2 counts landed against *live* tickets,
  // so 6 is the number and both surfaces read it from here.
  it('counts landed against live tickets, excluding dropped', () => {
    expect(progressText(seededBoard().progress)).toBe('1 of 6 landed · 1 in flight');
  });

  it('drops the in-flight clause when nothing is in flight', () => {
    const board = seededBoard();
    board.progress = { blocked: 0, ready: 0, in_flight: 0, landed: 5, dropped: 0, live: 5 };
    expect(progressText(board.progress)).toBe('5 of 5 landed');
  });

  // A bar that counted a run which finished without merging would contradict
  // the gate one screen below it (§6.4).
  it('does not count a closed-unmerged ticket as landed', () => {
    const board = seededBoard();
    board.progress = { blocked: 0, ready: 0, in_flight: 0, landed: 0, dropped: 1, live: 0 };
    board.tickets = [
      ticket(1, 'dropped', {
        feature: { id: 'f1', status: 'completed', mr_state: 'closed', mr_url: 'https://x/pull/9' },
      }),
    ];
    expect(progressText(board.progress)).toBe('0 of 0 landed');
  });

  it('says nothing at all before a decomposition has proposed anything', () => {
    expect(
      progressText({ blocked: 0, ready: 0, in_flight: 0, landed: 0, dropped: 0, live: 0 }),
    ).toBeNull();
  });
});

describe('progressSegments', () => {
  it('sizes both segments against the live denominator', () => {
    const { landedPct, inFlightPct } = progressSegments(seededBoard().progress);
    expect(landedPct).toBeCloseTo(100 / 6);
    expect(inFlightPct).toBeCloseTo(100 / 6);
  });

  it('is empty rather than infinite when every ticket was dropped', () => {
    expect(
      progressSegments({ blocked: 0, ready: 0, in_flight: 0, landed: 0, dropped: 1, live: 0 }),
    ).toEqual({ landedPct: 0, inFlightPct: 0 });
  });
});

describe('discoveryDetailLine', () => {
  it('names what is startable and what is waiting on a PR', () => {
    expect(discoveryDetailLine(seededBoard())).toBe(
      'DSC-4 is startable now. DSC-3 waits on PR #134.',
    );
  });

  it('says nothing when nothing is startable and nothing waits on a PR', () => {
    const board = seededBoard();
    board.tickets = [
      ticket(1, 'landed'),
      ticket(2, 'blocked', { blockedBy: ['t1'], unmet: ['t1'] }),
    ];
    expect(discoveryDetailLine(board)).toBeNull();
  });
});

describe('discoveryLifecycle', () => {
  const open: Discovery = {
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
  };

  it('pulses only while a turn is actually running', () => {
    expect(discoveryLifecycle(open, 7, true)).toEqual({
      label: 'Interviewing',
      tone: 'violet',
      live: true,
    });
    expect(discoveryLifecycle(open, 3, false)).toEqual({
      label: 'Decomposed',
      tone: 'cyan',
      live: false,
    });
    expect(discoveryLifecycle(open, 0, false)).toEqual({
      label: 'Interviewing',
      tone: 'violet',
      live: false,
    });
  });

  it('reads a closed discovery as closed however many tickets it left', () => {
    expect(discoveryLifecycle({ ...open, status: 'closed' }, 5, false)).toEqual({
      label: 'Closed',
      tone: 'slate',
      live: false,
    });
  });
});

describe('prNumber', () => {
  it('reads both forge spellings', () => {
    expect(prNumber('https://github.com/acme/app/pull/412')).toBe('412');
    expect(prNumber('https://gitlab.example.com/acme/app/-/merge_requests/77')).toBe('77');
  });

  it('is null for an unpublished run', () => {
    expect(prNumber(null)).toBeNull();
    expect(prNumber('https://github.com/acme/app')).toBeNull();
  });
});

describe('ticketLabel', () => {
  it('uses the stable seq, never a list index', () => {
    expect(ticketLabel(4)).toBe('DSC-4');
  });
});

// `docs/TASKS_DISCOVERY.md` recorded the Project Home card and the workspace
// header disagreeing about how many turns a Discovery had taken: the card read
// stored rows, the header counted rendered transcript blocks, and a turn
// carrying a question yields one more block than it does rows. A turn is one
// stored message, and both surfaces now say so through this.
describe('turnCountLabel', () => {
  it('counts stored messages, singular at one', () => {
    expect(turnCountLabel(1)).toBe('1 turn');
    expect(turnCountLabel(4)).toBe('4 turns');
  });

  it('says nothing has been said yet rather than nothing at all', () => {
    expect(turnCountLabel(0)).toBe('0 turns');
  });
});
