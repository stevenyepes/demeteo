import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { COALESCED_REFRESH_MIN_INTERVAL_MS, createCoalescedRefresh } from './coalescedRefresh';

interface Deferred {
  promise: Promise<void>;
  resolve: () => void;
  reject: (reason: Error) => void;
}

function deferred(): Deferred {
  let resolve!: () => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<void>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function manualRefresh(): { refresh: () => Promise<void>; calls: Deferred[] } {
  const calls: Deferred[] = [];
  return {
    calls,
    refresh: () => {
      const next = deferred();
      calls.push(next);
      return next.promise;
    },
  };
}

const cooldown = () => vi.advanceTimersByTimeAsync(COALESCED_REFRESH_MIN_INTERVAL_MS);

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('createCoalescedRefresh', () => {
  it('collapses a synchronous burst into one invocation', async () => {
    const { refresh, calls } = manualRefresh();
    const { trigger } = createCoalescedRefresh(refresh);

    for (let i = 0; i < 10; i += 1) trigger();

    expect(calls).toHaveLength(1);

    await cooldown();
    expect(calls).toHaveLength(1);
  });

  it('runs exactly one follow-up for the triggers that arrived during a call', async () => {
    const { refresh, calls } = manualRefresh();
    const { trigger } = createCoalescedRefresh(refresh);

    for (let i = 0; i < 10; i += 1) trigger();
    calls[0].resolve();
    await cooldown();

    expect(calls).toHaveLength(2);

    await cooldown();
    expect(calls).toHaveLength(2);
  });

  it('holds the queued call until the interval between starts has elapsed', async () => {
    const { refresh, calls } = manualRefresh();
    const { trigger } = createCoalescedRefresh(refresh);

    trigger();
    calls[0].resolve();
    trigger();
    await vi.advanceTimersByTimeAsync(COALESCED_REFRESH_MIN_INTERVAL_MS - 1);

    expect(calls).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(1);
    expect(calls).toHaveLength(2);
  });

  it('invokes immediately when nothing is in flight and the cooldown has elapsed', async () => {
    const { refresh, calls } = manualRefresh();
    const { trigger } = createCoalescedRefresh(refresh);

    trigger();
    calls[0].resolve();
    await cooldown();

    trigger();
    expect(calls).toHaveLength(2);
  });

  it('is not wedged by a rejected invocation', async () => {
    const { refresh, calls } = manualRefresh();
    const { trigger } = createCoalescedRefresh(refresh);

    trigger();
    calls[0].reject(new Error('IPC down'));
    await cooldown();

    trigger();
    expect(calls).toHaveLength(2);
  });

  it('is not wedged by a synchronously throwing invocation', async () => {
    const calls: string[] = [];
    let explode = true;
    const refresh = (): Promise<void> => {
      calls.push('call');
      if (explode) throw new Error('IPC down');
      return Promise.resolve();
    };
    const { trigger } = createCoalescedRefresh(refresh);

    trigger();
    expect(calls).toHaveLength(1);

    explode = false;
    await cooldown();
    trigger();

    expect(calls).toHaveLength(2);
  });

  it('measures the cooldown on a monotonic clock, not the wall clock', async () => {
    const { refresh, calls } = manualRefresh();
    const { trigger } = createCoalescedRefresh(refresh);

    trigger();
    calls[0].resolve();
    vi.setSystemTime(Date.now() - 3_600_000);
    trigger();
    await cooldown();

    expect(calls).toHaveLength(2);
  });

  it('cancels a queued invocation when disposed', async () => {
    const { refresh, calls } = manualRefresh();
    const scheduler = createCoalescedRefresh(refresh);

    scheduler.trigger();
    calls[0].resolve();
    scheduler.trigger();
    await vi.advanceTimersByTimeAsync(1);

    expect(calls).toHaveLength(1);

    scheduler.dispose();
    await cooldown();

    expect(calls).toHaveLength(1);
  });
});
