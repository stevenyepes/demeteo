import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useThrottledValue } from './useThrottledValue';

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('a throttled value', () => {
  it('holds the published value while the producer runs ahead', () => {
    const { result, rerender } = renderHook(({ text }) => useThrottledValue(text, 250), {
      initialProps: { text: 'C' },
    });

    for (const text of ['Ch', 'Che', 'Chec', 'Check']) {
      rerender({ text });
      act(() => void vi.advanceTimersByTime(16));
    }

    expect(result.current).toBe('C');
  });

  it('publishes the latest value once the interval is up, not the one that scheduled it', () => {
    const { result, rerender } = renderHook(({ text }) => useThrottledValue(text, 250), {
      initialProps: { text: 'C' },
    });

    rerender({ text: 'Ch' });
    act(() => void vi.advanceTimersByTime(100));
    rerender({ text: 'Checked' });
    act(() => void vi.advanceTimersByTime(250));

    expect(result.current).toBe('Checked');
  });

  it('never strands the last value', () => {
    const { result, rerender } = renderHook(({ text }) => useThrottledValue(text, 250), {
      initialProps: { text: 'a' },
    });

    act(() => void vi.advanceTimersByTime(1000));
    rerender({ text: 'b' });
    act(() => void vi.advanceTimersByTime(250));

    expect(result.current).toBe('b');
  });
});
