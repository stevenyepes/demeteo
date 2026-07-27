/**
 * `useWorkflowLint` (task P3.3): the debounce contract. Three claims, each one
 * a bug the builder would otherwise have — an IPC per keystroke, badges that
 * blink off mid-edit, and a slow early lint overwriting a fast later one.
 */
import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import { useWorkflowLint } from './useWorkflowLint';
import type { LintFinding } from './lint';
import type { WorkflowDefinitionV2 } from './types';

const def = (title: string): WorkflowDefinitionV2 => ({
  schema_version: 2,
  id: 'wf-w',
  name: 'W',
  nodes: [{ id: 'plan', type: 'agent', title }],
  edges: [],
});

const finding = (code: string): LintFinding => ({
  severity: 'error',
  code,
  node: 'plan',
  message: code,
});

// Block bodies on purpose: an arrow returning the mock would hand vitest a
// *function* as the hook's teardown, which it then calls — invoking `invoke`
// after the test and rejecting into nobody's `catch`.
beforeEach(() => {
  vi.mocked(invoke).mockReset();
});
afterEach(() => {
  vi.useRealTimers();
});

describe('useWorkflowLint', () => {
  it('lints once after the debounce, not once per edit', async () => {
    vi.useFakeTimers();
    vi.mocked(invoke).mockResolvedValue([finding('missing-prompt')]);

    const { result, rerender } = renderHook(({ d }) => useWorkflowLint(d, 300), {
      initialProps: { d: def('a') },
    });

    rerender({ d: def('b') });
    rerender({ d: def('c') });
    expect(invoke).not.toHaveBeenCalled(); // still inside the window

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('workflow_lint', { definition: def('c') });
    expect(result.current.lint.hasErrors).toBe(true);
    expect(result.current.checking).toBe(false);
  });

  it('skips the round-trip when a re-render produces an identical graph', async () => {
    vi.useFakeTimers();
    vi.mocked(invoke).mockResolvedValue([]);

    const { rerender } = renderHook(({ d }) => useWorkflowLint(d, 300), {
      initialProps: { d: def('a') },
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(invoke).toHaveBeenCalledTimes(1);

    // A fresh object with the same content — what the canvas hands us on most
    // renders.
    rerender({ d: def('a') });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it('keeps the previous findings visible while a new lint is in flight', async () => {
    let release: (f: LintFinding[]) => void = () => {};
    vi.mocked(invoke)
      .mockResolvedValueOnce([finding('first')])
      .mockImplementationOnce(() => new Promise((resolve) => (release = resolve)));

    const { result, rerender } = renderHook(({ d }) => useWorkflowLint(d, 0), {
      initialProps: { d: def('a') },
    });
    await waitFor(() => expect(result.current.lint.errors).toHaveLength(1));
    expect(result.current.lint.errors[0].code).toBe('first');

    rerender({ d: def('b') });
    await waitFor(() => expect(result.current.checking).toBe(true));
    // Stale but present — badges don't blink off mid-edit.
    expect(result.current.lint.errors[0].code).toBe('first');

    await act(async () => {
      release([finding('second')]);
    });
    await waitFor(() => expect(result.current.lint.errors[0].code).toBe('second'));
  });

  it('drops a reply that lost the race to a newer one', async () => {
    let releaseSlow: (f: LintFinding[]) => void = () => {};
    vi.mocked(invoke)
      .mockImplementationOnce(() => new Promise((resolve) => (releaseSlow = resolve)))
      .mockResolvedValueOnce([finding('newer')]);

    const { result, rerender } = renderHook(({ d }) => useWorkflowLint(d, 0), {
      initialProps: { d: def('a') },
    });
    // Let the first request go out, then supersede it.
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    rerender({ d: def('b') });
    await waitFor(() => expect(result.current.lint.errors[0]?.code).toBe('newer'));

    await act(async () => {
      releaseSlow([finding('stale')]);
    });
    // The late answer to an old question is discarded.
    expect(result.current.lint.errors[0].code).toBe('newer');
  });

  it('reports a command failure without pretending the graph is clean', async () => {
    vi.mocked(invoke).mockImplementation(() => Promise.reject('backend exploded'));
    const { result } = renderHook(() => useWorkflowLint(def('a'), 0));
    await waitFor(() => expect(result.current.error).toContain('backend exploded'));
    expect(result.current.lint.hasErrors).toBe(false);
  });
});
