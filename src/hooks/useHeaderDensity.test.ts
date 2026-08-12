/**
 * The two branches of `useHeaderDensity` that `TopBar.test.tsx` cannot reach
 * through the component: the host that has no `ResizeObserver`, and the
 * teardown. Both are invisible from a mounted `TopBar` — the first because the
 * test host always provides the stub, the second because nothing rendered
 * changes when an observer is left attached.
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { useHeaderDensity } from './useHeaderDensity';
import { HEADER_ICONS_BELOW_PX } from '../lib/headerLayout';
import { resizeObserverStubs } from '../test/setup';

const installed = globalThis.ResizeObserver;

afterEach(() => {
  globalThis.ResizeObserver = installed;
});

function element(width: number): HTMLElement {
  const el = document.createElement('header');
  Object.defineProperty(el, 'offsetWidth', { configurable: true, value: width });
  return el;
}

describe('useHeaderDensity', () => {
  it('pins the density where no ResizeObserver exists', () => {
    // A host with no observer cannot be measured, so the seed is the answer for
    // the lifetime of the app rather than for one frame — `labels` is what the
    // header showed before the ladder existed.
    (globalThis as { ResizeObserver?: typeof ResizeObserver }).ResizeObserver = undefined;

    const { result } = renderHook(() => useHeaderDensity());
    act(() => result.current.setHeaderEl(element(800)));

    expect(resizeObserverStubs).toHaveLength(0);
    expect(result.current.density).toBe('labels');
  });

  it('disconnects the observer when the header goes away', () => {
    const { result, unmount } = renderHook(() => useHeaderDensity());
    const first = element(HEADER_ICONS_BELOW_PX - 1);
    act(() => result.current.setHeaderEl(first));

    const observer = resizeObserverStubs.find((o) =>
      o.observe.mock.calls.some(([target]) => target === first),
    );
    if (!observer) throw new Error('no ResizeObserver was registered for the header');
    act(() => observer.trigger());
    expect(result.current.density).toBe('icons');

    // A second element re-arms the effect; the observer for the first has to be
    // released, or a detached node keeps answering for a header that is gone.
    act(() => result.current.setHeaderEl(element(2000)));
    expect(observer.disconnect).toHaveBeenCalledTimes(1);

    unmount();
    const live = resizeObserverStubs.filter((o) => o.disconnect.mock.calls.length === 0);
    expect(live).toEqual([]);
  });
});
