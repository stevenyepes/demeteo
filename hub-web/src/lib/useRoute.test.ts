import { renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { useRoute } from './useRoute';

afterEach(() => {
  vi.restoreAllMocks();
  window.location.hash = '';
});

function hashchangeListeners(spy: { mock: { calls: unknown[][] } }): unknown[] {
  return spy.mock.calls.filter(([type]) => type === 'hashchange').map(([, listener]) => listener);
}

describe('useRoute', () => {
  it('returns the route for the hash present at mount', () => {
    window.location.hash = '#/instances/i-1/runs/f-9';

    const { result } = renderHook(() => useRoute());

    expect(result.current).toEqual({ name: 'run', instanceId: 'i-1', featureId: 'f-9' });
  });

  it('reads an empty hash as fleet', () => {
    const { result } = renderHook(() => useRoute());

    expect(result.current).toEqual({ name: 'fleet' });
  });

  it('returns the new route after the hash changes', async () => {
    window.location.hash = '#/runs';
    const { result } = renderHook(() => useRoute());
    expect(result.current).toEqual({ name: 'runs' });

    window.location.hash = '#/instances/i-1';

    // jsdom queues `hashchange` as a task, so the route is still the old one
    // on the line after the assignment.
    await waitFor(() => expect(result.current).toEqual({ name: 'instance', instanceId: 'i-1' }));
  });

  it('keeps the same route object across a re-render that did not move the hash', () => {
    window.location.hash = '#/instances/i-1';
    const { result, rerender } = renderHook(() => useRoute());
    const first = result.current;

    rerender();

    expect(result.current).toBe(first);
  });

  it('leaves no hashchange listener behind after unmount', () => {
    const added = vi.spyOn(window, 'addEventListener');
    const removed = vi.spyOn(window, 'removeEventListener');

    const { unmount } = renderHook(() => useRoute());
    const subscribed = hashchangeListeners(added);
    expect(subscribed).toHaveLength(1);
    expect(hashchangeListeners(removed)).toEqual([]);

    unmount();

    expect(hashchangeListeners(removed)).toEqual(subscribed);
  });
});
