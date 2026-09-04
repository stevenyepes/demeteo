/**
 * `useDiscoveryColumnLayout` measures via `el.offsetWidth` / `offsetHeight`
 * inside a no-argument observer callback, same as `useHeaderDensity` —
 * jsdom's `ResizeObserverStub` fires with an empty entry list, so a test that
 * reads `entry.contentRect` could never reach this hook at all.
 */
import { act, renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { useDiscoveryColumnLayout } from './useDiscoveryColumnLayout';
import { resizeObserverStubs } from '../../test/setup';

function element(width: number, height = 800): HTMLDivElement {
  const el = document.createElement('div');
  Object.defineProperty(el, 'offsetWidth', { configurable: true, value: width });
  Object.defineProperty(el, 'offsetHeight', { configurable: true, value: height });
  return el;
}

function observerFor(target: HTMLDivElement) {
  const observer = resizeObserverStubs.find((o) => o.observe.mock.calls.some(([t]) => t === target));
  if (!observer) throw new Error('no ResizeObserver was registered for the row');
  return observer;
}

describe('useDiscoveryColumnLayout', () => {
  it('crosses three-up -> overlay-inspector -> stacked purely from triggered resizes', () => {
    const { result } = renderHook(() => useDiscoveryColumnLayout());

    const wide = element(1400);
    act(() => result.current.setRowEl(wide));
    act(() => observerFor(wide).trigger());
    expect(result.current.layoutMode).toBe('three-up');

    const medium = element(1000);
    act(() => result.current.setRowEl(medium));
    act(() => observerFor(medium).trigger());
    expect(result.current.layoutMode).toBe('overlay-inspector');

    const narrow = element(600);
    act(() => result.current.setRowEl(narrow));
    act(() => observerFor(narrow).trigger());
    expect(result.current.layoutMode).toBe('stacked');
  });

  it('disconnects the observer when the row element changes or the hook unmounts', () => {
    const { result, unmount } = renderHook(() => useDiscoveryColumnLayout());

    const first = element(1400);
    act(() => result.current.setRowEl(first));
    const firstObserver = observerFor(first);
    act(() => firstObserver.trigger());
    expect(result.current.layoutMode).toBe('three-up');

    const second = element(600);
    act(() => result.current.setRowEl(second));
    expect(firstObserver.disconnect).toHaveBeenCalledTimes(1);

    unmount();
    const live = resizeObserverStubs.filter((o) => o.disconnect.mock.calls.length === 0);
    expect(live).toEqual([]);
  });
});
