import { describe, expect, it } from 'vitest';

import {
  LOCK_NAMES_INLINE,
  baseBranchLabel,
  baseBranchLock,
  resolveBaseBranchInput,
} from './discoveryBase';
import type { Ticket, TicketState } from '../types';

function ticket(seq: number, state: TicketState): Ticket {
  return {
    id: `t${seq}`,
    discovery_id: 'd1',
    seq,
    title: `Ticket ${seq}`,
    description: '',
    acceptance: [],
    files: [],
    blocked_by: [],
    test_command: null,
    workflow_id: null,
    agent_kind: null,
    model: null,
    effort: null,
    machine_id: null,
    attachments: [],
    state,
    drop_reason: null,
    force_start_reason: null,
    force_started_at: null,
    feature_id: null,
    created_at: 0,
    updated_at: 0,
  };
}

describe('baseBranchLock', () => {
  const unlocked = { locked: false, tickets: [], summary: null, listed: false };

  it('is unlocked when there are no tickets', () => {
    expect(baseBranchLock([])).toEqual(unlocked);
  });

  it('is unlocked while every ticket is unstarted', () => {
    expect(baseBranchLock([ticket(1, 'unstarted'), ticket(2, 'unstarted')])).toEqual(unlocked);
  });

  it('locks once a ticket has started, and names it', () => {
    const lock = baseBranchLock([ticket(1, 'unstarted'), ticket(3, 'started')]);
    expect(lock.locked).toBe(true);
    expect(lock.tickets).toEqual(['DSC-3']);
    expect(lock.summary).toBe('Base branch locked: DSC-3 has left Unstarted.');
  });

  it('locks equally on a dropped ticket, not just a started one', () => {
    const lock = baseBranchLock([ticket(2, 'dropped')]);
    expect(lock.locked).toBe(true);
    expect(lock.summary).toContain('DSC-2');
  });

  it('names a few tickets inline', () => {
    const lock = baseBranchLock([ticket(1, 'started'), ticket(2, 'dropped')]);
    expect(lock.listed).toBe(true);
    expect(lock.summary).toBe('Base branch locked: DSC-1, DSC-2 have left Unstarted.');
  });

  it('counts rather than names once there are more than fit in a line', () => {
    const many = Array.from({ length: LOCK_NAMES_INLINE + 1 }, (_, i) => ticket(i + 1, 'started'));
    const lock = baseBranchLock(many);
    expect(lock.listed).toBe(false);
    expect(lock.summary).toBe(
      `Base branch locked: ${LOCK_NAMES_INLINE + 1} tickets have left Unstarted.`,
    );
    expect(lock.summary).not.toContain('DSC-1');
    expect(lock.tickets).toHaveLength(LOCK_NAMES_INLINE + 1);
  });
});

describe('baseBranchLabel', () => {
  it('renders the base branch verbatim when set', () => {
    expect(baseBranchLabel('feat/x', 'main')).toBe('feat/x');
    expect(baseBranchLabel('feat/x', 'main')).not.toBe('');
  });

  it('names the known default branch when unset', () => {
    const label = baseBranchLabel(null, 'main');
    expect(label).not.toBe('');
    expect(label).toContain('main');
  });

  it('falls back to a generic phrase when neither is known, never blank', () => {
    const label = baseBranchLabel(null, null);
    expect(label).not.toBe('');
    expect(label).not.toBe('null');
    expect(label.toLowerCase()).toContain("project's default branch");
  });
});

describe('resolveBaseBranchInput', () => {
  it('returns null for the project default regardless of the name field', () => {
    expect(resolveBaseBranchInput(true, 'anything')).toBeNull();
    expect(resolveBaseBranchInput(true, '')).toBeNull();
  });

  it('returns the trimmed name for a named branch', () => {
    expect(resolveBaseBranchInput(false, ' feat/x ')).toBe('feat/x');
  });

  it('returns null when a named branch trims to empty', () => {
    expect(resolveBaseBranchInput(false, '  ')).toBeNull();
  });
});
