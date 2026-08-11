import { describe, expect, it } from 'vitest';

import { activitySync, type ActivitySyncInput } from './activitySync';

function sync(overrides: Partial<ActivitySyncInput> = {}) {
  return activitySync({
    transport: 'remote',
    open: true,
    terminal: false,
    pollMs: 2000,
    ...overrides,
  });
}

describe('activitySync', () => {
  it('names the interval the poll actually runs at', () => {
    expect(sync({ pollMs: 2000 }).label).toBe('every 2s');
    expect(sync({ pollMs: 5000 }).label).toBe('every 5s');
  });

  it('pulses only while the feed is advancing', () => {
    expect(sync().live).toBe(true);
    expect(sync({ open: false }).live).toBe(false);
    expect(sync({ terminal: true }).live).toBe(false);
    expect(sync({ consecutiveFailures: 1 }).live).toBe(false);
    expect(sync({ errored: true }).live).toBe(false);
  });

  it('says a closed remote panel has stopped polling', () => {
    const closed = sync({ open: false });

    expect(closed.label).toBe('paused');
    expect(closed.tone).toBe('slate');
    expect(closed.hint).toMatch(/not being polled/);
  });

  it('does not call a closed local panel paused — the push does not stop', () => {
    const closed = sync({ transport: 'local', open: false });

    expect(closed.label).toBe('live');
    expect(closed.live).toBe(true);
    expect(closed.hint).toMatch(/keep arriving/);
  });

  it('separates one dropped poll from a streak long enough to show an error', () => {
    expect(sync({ consecutiveFailures: 1 })).toMatchObject({ label: 'reconnecting', tone: 'amber' });
    expect(sync({ consecutiveFailures: 3, errored: true })).toMatchObject({
      label: 'disconnected',
      tone: 'ruby',
    });
  });

  it('still names the retry interval while disconnected', () => {
    expect(sync({ consecutiveFailures: 3, errored: true, pollMs: 2000 }).hint).toMatch(/every 2s/);
  });

  it('reports a finished run as final on either transport', () => {
    expect(sync({ terminal: true })).toMatchObject({ label: 'final', tone: 'slate' });
    expect(sync({ transport: 'local', terminal: true })).toMatchObject({
      label: 'final',
      tone: 'slate',
    });
  });

  it('prefers final over a failure streak — a fetched-once log cannot reconnect', () => {
    expect(sync({ terminal: true, errored: true }).label).toBe('final');
  });
});
