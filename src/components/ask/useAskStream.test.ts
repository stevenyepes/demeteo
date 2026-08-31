/**
 * `useAskStream` mirrors `discovery/useDiscoveryStream.ts`, keyed by
 * `thread_id`. These prove per-thread isolation, `NO_TURN` identity across a
 * no-op fold, and `begin`/`end` phase semantics.
 */
import { renderHook, act, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

// Capture `listen` handlers so a test can dispatch synthetic Tauri events.
const handlers: Record<string, Array<(e: { payload: unknown }) => void>> = {};
vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => {
    (handlers[event] ??= []).push(cb);
    return Promise.resolve(() => {
      handlers[event] = (handlers[event] ?? []).filter((h) => h !== cb);
    });
  },
}));

import { useAskStream, useStreamedTurn } from './useAskStream';
import { NO_TURN } from '../../lib/askActivity';
import { EVENT_ASK_AGENT_EVENT, type AskAgentEventPayload } from '../../lib/ask';

function emit(threadId: string, event: unknown) {
  const payload: AskAgentEventPayload = { thread_id: threadId, event };
  for (const h of handlers[EVENT_ASK_AGENT_EVENT] ?? []) h({ payload });
}

const textEvent = (delta: string) => ({ kind: 'text', delta });

afterEach(() => {
  for (const k of Object.keys(handlers)) delete handlers[k];
});

describe('useAskStream', () => {
  it('keeps one LiveTurn per thread_id — events on one thread do not leak into another', async () => {
    const { result } = renderHook(() => useAskStream());
    await waitFor(() => expect(handlers[EVENT_ASK_AGENT_EVENT]?.length).toBeGreaterThan(0));

    const a = renderHook(() => useStreamedTurn(result.current.store, 'thread-a')).result;
    const b = renderHook(() => useStreamedTurn(result.current.store, 'thread-b')).result;

    act(() => emit('thread-a', textEvent('hello')));
    await waitFor(() => expect(a.current.text).toBe('hello'));
    expect(b.current.text).toBe('');
    expect(b.current).toBe(NO_TURN);
  });

  it('returns NO_TURN by reference for a thread with no turn', () => {
    const { result } = renderHook(() => useAskStream());
    const { result: streamed } = renderHook(() => useStreamedTurn(result.current.store, 'thread-x'));
    expect(Object.is(streamed.current, NO_TURN)).toBe(true);
  });

  it('returns the same LiveTurn reference across a fold that changes nothing', async () => {
    const { result } = renderHook(() => useAskStream());
    await waitFor(() => expect(handlers[EVENT_ASK_AGENT_EVENT]?.length).toBeGreaterThan(0));

    const { result: streamed } = renderHook(() => useStreamedTurn(result.current.store, 'thread-y'));

    act(() => result.current.begin('thread-y', 'working'));
    await waitFor(() => expect(streamed.current.startedAt).toBeGreaterThan(0));
    const opened = streamed.current;

    // An empty text delta folds to the same string, so `foldTurnEvent` hands
    // back `turn` itself — `useSyncExternalStore` depends on that reference
    // staying stable so a no-op event doesn't wake a subscriber.
    await act(async () => {
      emit('thread-y', textEvent(''));
      await new Promise((resolve) => requestAnimationFrame(resolve));
    });

    expect(Object.is(streamed.current, opened)).toBe(true);
  });

  it('preserves the NO_TURN reference for an untouched thread across reads', () => {
    const { result } = renderHook(() => useAskStream());
    const first = result.current.store.read('thread-untouched');
    const second = result.current.store.read('thread-untouched');
    expect(Object.is(first, NO_TURN)).toBe(true);
    expect(Object.is(second, NO_TURN)).toBe(true);
  });

  it('begin opens a turn with the given phase, idempotent on a repeated phase', async () => {
    const { result } = renderHook(() => useAskStream());
    const { result: streamed } = renderHook(() => useStreamedTurn(result.current.store, 'thread-z'));

    act(() => result.current.begin('thread-z', 'setting_up'));
    await waitFor(() => expect(streamed.current.phase).toBe('setting_up'));
    expect(streamed.current.startedAt).toBeGreaterThan(0);
    const startedAt = streamed.current.startedAt;

    act(() => result.current.begin('thread-z', 'setting_up'));
    expect(streamed.current.startedAt).toBe(startedAt);

    act(() => result.current.begin('thread-z', 'working'));
    await waitFor(() => expect(streamed.current.phase).toBe('working'));
    expect(streamed.current.startedAt).toBe(startedAt);
  });

  it('end clears a thread turn back to NO_TURN', async () => {
    const { result } = renderHook(() => useAskStream());
    const { result: streamed } = renderHook(() => useStreamedTurn(result.current.store, 'thread-w'));

    act(() => result.current.begin('thread-w', 'working'));
    await waitFor(() => expect(streamed.current.phase).toBe('working'));

    act(() => result.current.end('thread-w'));
    await waitFor(() => expect(Object.is(streamed.current, NO_TURN)).toBe(true));
  });
});
