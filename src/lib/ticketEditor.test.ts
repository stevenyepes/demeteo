// `docs/PRD_DISCOVERY.md` §5.4 locks a Ticket the moment it has a Feature, and
// `application::tickets::is_locked` reads *either* half — the column or the
// state. These pin both, because the drawer shows a locked ticket as locked
// rather than letting the save fail one round trip later.
//
// The whole-payload rule is pinned here too: every key of `TicketEdit` is
// required on the wire, and Rust reads an absent key and an explicit `null`
// identically, so a payload that dropped one would silently mean *keep* where
// the user meant *clear*.

import { describe, expect, it } from 'vitest';

import { draftOf, edgeOptions, editOf, isDirty, isTicketLocked, stagedCount } from './ticketEditor';
import type { Ticket, TicketView } from '../types';

function ticket(extra: Partial<Ticket> = {}): Ticket {
  return {
    id: 't1',
    discovery_id: 'dsc-1',
    seq: 3,
    title: 'Multiplex run streams over one connection',
    description: 'Every client watches its own runs down a single connection.',
    acceptance: ['Two clients stream concurrently without interleaving'],
    files: ['crates/demeteo-runner/src/stream/mux.rs'],
    blocked_by: ['t2'],
    test_command: 'npm run checks:code',
    workflow_id: 'wf-1',
    agent_kind: 'claude-code',
    model: 'opus',
    effort: 'high',
    attachments: [],
    state: 'unstarted',
    drop_reason: null,
    force_start_reason: null,
    force_started_at: null,
    feature_id: null,
    created_at: 0,
    updated_at: 0,
    ...extra,
  };
}

function view(row: Ticket): TicketView {
  return {
    ticket: row,
    standing: { id: row.id, lane: 'ready', startable: true, blockers: [] },
    feature: null,
  };
}

describe('the locked rule', () => {
  it('leaves an unstarted ticket editable', () => {
    expect(isTicketLocked(ticket())).toBe(false);
  });

  it('locks a ticket that has a feature', () => {
    expect(isTicketLocked(ticket({ feature_id: 'f-1' }))).toBe(true);
  });

  it('locks a started ticket whose feature id has not been read back yet', () => {
    expect(isTicketLocked(ticket({ state: 'started' }))).toBe(true);
  });

  it('leaves a dropped ticket editable — a re-decomposition may revise one', () => {
    expect(isTicketLocked(ticket({ state: 'dropped', drop_reason: 'folded in' }))).toBe(false);
  });
});

describe('the save payload', () => {
  it('carries every key, so an omitted one can never mean "keep"', () => {
    expect(Object.keys(editOf(draftOf(ticket()))).sort()).toEqual([
      'acceptance',
      'agent_kind',
      'blocked_by',
      'description',
      'effort',
      'files',
      'model',
      'test_command',
      'title',
      'workflow_id',
    ]);
  });

  it('sends an explicit null for a field the user cleared', () => {
    const draft = { ...draftOf(ticket()), model: '', testCommand: '   ' };

    expect(editOf(draft).model).toBeNull();
    expect(editOf(draft).test_command).toBeNull();
  });

  it('drops the blank rows a form leaves behind', () => {
    const draft = { ...draftOf(ticket()), acceptance: ['first', '  ', ''] };

    expect(editOf(draft).acceptance).toEqual(['first']);
  });

  it('round-trips a row it has not touched', () => {
    const row = ticket();

    expect(editOf(draftOf(row))).toEqual({
      title: row.title,
      description: row.description,
      acceptance: row.acceptance,
      files: row.files,
      blocked_by: row.blocked_by,
      test_command: row.test_command,
      workflow_id: row.workflow_id,
      agent_kind: row.agent_kind,
      model: row.model,
      effort: row.effort,
    });
  });
});

describe('dirtiness', () => {
  it('is false on an untouched draft', () => {
    expect(isDirty(draftOf(ticket()), ticket())).toBe(false);
  });

  it('ignores whitespace the user only walked through', () => {
    expect(isDirty({ ...draftOf(ticket()), title: '  Multiplex run streams over one connection  ' }, ticket())).toBe(
      false,
    );
  });

  it('is true once a field actually differs', () => {
    expect(isDirty({ ...draftOf(ticket()), model: 'qwen3-coder-480b' }, ticket())).toBe(true);
  });
});

describe('the edge picker', () => {
  const rows = [view(ticket()), view(ticket({ id: 't2', seq: 2 })), view(ticket({ id: 't4', seq: 4 }))];

  it('offers the siblings, never the ticket itself', () => {
    expect(edgeOptions(ticket(), rows, []).map((option) => option.id)).toEqual(['t2', 't4']);
  });

  it('drops the ones already chosen', () => {
    expect(edgeOptions(ticket(), rows, ['t2']).map((option) => option.label)).toEqual(['DSC-4']);
  });
});

describe('the staged chip', () => {
  it('counts against the dropzone ceiling', () => {
    expect(stagedCount(0)).toBe('0 of 10 · staged');
    expect(stagedCount(3)).toBe('3 of 10 · staged');
  });
});
