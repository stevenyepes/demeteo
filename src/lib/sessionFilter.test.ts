/**
 * The session filter policy, over plain rows — no component, because the
 * whole point of `lib/sessionFilter.ts` is that "which rows survive" is
 * answerable without one.
 */
import { describe, expect, it } from 'vitest';

import {
  agentKinds,
  clearFilters,
  filterSessions,
  isNarrowed,
  segmentCounts,
  ANY_AGENT_KIND,
  DEFAULT_SESSION_FILTER,
  type SessionFilterOptions,
  type SessionRow,
} from './sessionFilter';

function row(overrides: Partial<SessionRow> = {}): SessionRow {
  return {
    title: 'A thread',
    status: 'open',
    agent_kind: 'opencode',
    created_at: 1_000,
    updated_at: 1_000,
    ...overrides,
  };
}

function options(overrides: Partial<SessionFilterOptions> = {}): SessionFilterOptions {
  return { ...DEFAULT_SESSION_FILTER, ...overrides };
}

describe('segmentCounts', () => {
  it('tallies every segment, with all as the total', () => {
    const rows = [row(), row({ status: 'closed' }), row({ status: 'closed' })];
    expect(segmentCounts(rows)).toEqual({ all: 3, open: 1, closed: 2 });
  });
});

describe('agentKinds', () => {
  it('is the distinct set present, sorted, so no option matches nothing', () => {
    const rows = [row({ agent_kind: 'opencode' }), row({ agent_kind: 'codex' }), row({ agent_kind: 'codex' })];
    expect(agentKinds(rows)).toEqual(['codex', 'opencode']);
  });
});

describe('filterSessions', () => {
  it('keeps only the chosen status', () => {
    const open = row({ title: 'open one' });
    const closed = row({ title: 'closed one', status: 'closed' });
    expect(filterSessions([open, closed], options({ segment: 'closed' }))).toEqual([closed]);
  });

  it('keeps only the chosen agent kind', () => {
    const a = row({ title: 'a', agent_kind: 'codex' });
    const b = row({ title: 'b', agent_kind: 'claude-code' });
    expect(filterSessions([a, b], options({ agentKind: 'claude-code' }))).toEqual([b]);
  });

  it('matches every whitespace-separated term as a substring of the title', () => {
    const hit = row({ title: 'Discovery session user journey' });
    const miss = row({ title: 'Runner keepalive' });
    expect(filterSessions([hit, miss], options({ query: 'journey discovery' }))).toEqual([hit]);
  });

  it('sorts recent on updated_at and newest on created_at — they are different questions', () => {
    const older = row({ title: 'older', created_at: 1, updated_at: 100 });
    const newer = row({ title: 'newer', created_at: 50, updated_at: 2 });

    expect(filterSessions([newer, older], options({ sort: 'recent' })).map((r) => r.title)).toEqual([
      'older',
      'newer',
    ]);
    expect(filterSessions([older, newer], options({ sort: 'newest' })).map((r) => r.title)).toEqual([
      'newer',
      'older',
    ]);
    expect(filterSessions([newer, older], options({ sort: 'oldest' })).map((r) => r.title)).toEqual([
      'older',
      'newer',
    ]);
  });

  it('holds input order for rows whose sort key ties', () => {
    const first = row({ title: 'first', updated_at: 7 });
    const second = row({ title: 'second', updated_at: 7 });
    expect(filterSessions([first, second], options()).map((r) => r.title)).toEqual([
      'first',
      'second',
    ]);
  });

  it('returns the input array identity when nothing was dropped and nothing moved', () => {
    const rows = [row({ title: 'a', updated_at: 9 }), row({ title: 'b', updated_at: 3 })];
    expect(filterSessions(rows, options())).toBe(rows);
  });
});

describe('clearFilters', () => {
  it('drops the hiding choices and keeps the ordering one', () => {
    const narrowed = options({
      segment: 'closed',
      query: 'x',
      agentKind: 'codex',
      sort: 'oldest',
    });
    expect(clearFilters(narrowed)).toEqual(options({ sort: 'oldest' }));
  });

  it('returns the same object when nothing narrows', () => {
    const wide = options({ sort: 'newest' });
    expect(clearFilters(wide)).toBe(wide);
  });
});

describe('isNarrowed', () => {
  it('counts every hiding control and no ordering one', () => {
    expect(isNarrowed(options())).toBe(false);
    expect(isNarrowed(options({ sort: 'oldest' }))).toBe(false);
    expect(isNarrowed(options({ segment: 'open' }))).toBe(true);
    expect(isNarrowed(options({ query: 'a' }))).toBe(true);
    expect(isNarrowed(options({ agentKind: 'codex' }))).toBe(true);
    expect(isNarrowed(options({ agentKind: ANY_AGENT_KIND }))).toBe(false);
  });
});
