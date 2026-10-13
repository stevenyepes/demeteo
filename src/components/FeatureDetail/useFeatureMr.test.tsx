import { invoke } from '@tauri-apps/api/core';
import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { ProjectProvider, useProject } from '../../context';
import { useFeatureMr } from './useFeatureMr';

/** Rejects anything it was not told to answer, so a hook that skipped the
 *  rollup cannot pass against a default. */
function backend(answers: Record<string, () => unknown>): void {
  vi.mocked(invoke).mockImplementation((cmd: string) =>
    cmd in answers ? Promise.resolve(answers[cmd]()) : Promise.reject(new Error(`unexpected command ${cmd}`)),
  );
}

describe('useFeatureMr cleanup', () => {
  it('clears the rail badge of the stuck gate it archived', async () => {
    backend({
      feature_get: () => ({ id: 'f-1', mr_url: null, mr_state: 'none' }),
      feature_cleanup: () => ({ policy: 'archive', action: 'archived', warnings: [] }),
      feature_status_rollup: () => [],
    });
    const navigate = vi.fn();
    const { result } = renderHook(
      () => ({
        mr: useFeatureMr({ featureId: 'f-1', projectId: 'proj-1', status: 'gated', reload: vi.fn(), navigate }),
        project: useProject(),
      }),
      { wrapper: ProjectProvider },
    );
    act(() =>
      result.current.project.dispatch({
        type: 'SET_PROJECT_ACTIVITY',
        activity: { 'proj-1': { active: 0, needsYou: 1 } },
      }),
    );

    await act(async () => {
      await result.current.mr.handleCleanup();
    });

    await waitFor(() => expect(result.current.project.state.activityByProject).toEqual({}));
    expect(navigate).toHaveBeenCalledWith({ kind: 'home' });
  });
});

describe('useFeatureMr publish', () => {
  it('shows the link as soon as the publish returns, without a status change', async () => {
    backend({
      feature_get: () => ({ id: 'f-1', mr_url: null, mr_state: 'none' }),
      publish_mr: () => ({
        kind: 'opened',
        mr: {
          url: 'https://github.com/o/r/pull/7',
          state: 'open',
          number: 7,
          provider_kind: 'github',
          provider_host: 'github.com',
        },
      }),
    });
    const { result } = renderHook(
      () => useFeatureMr({ featureId: 'f-1', projectId: 'proj-1', status: 'completed', reload: vi.fn(), navigate: vi.fn() }),
      { wrapper: ProjectProvider },
    );
    await waitFor(() => expect(result.current.mrState).toBe('none'));

    await act(async () => {
      await result.current.handlePublishClick();
    });

    expect(result.current.mrUrl).toBe('https://github.com/o/r/pull/7');
    expect(result.current.mrState).toBe('open');
  });

  it('leaves the link alone when the runner holding the run publishes it', async () => {
    backend({
      feature_get: () => ({ id: 'f-1', mr_url: null, mr_state: 'none' }),
      publish_mr: () => ({ kind: 'on_runner', machine_id: 'm-1', run_id: 'run-1' }),
    });
    const reload = vi.fn();
    const { result } = renderHook(
      () => useFeatureMr({ featureId: 'f-1', projectId: 'proj-1', status: 'awaiting_mr', reload, navigate: vi.fn() }),
      { wrapper: ProjectProvider },
    );
    await waitFor(() => expect(result.current.mrState).toBe('none'));

    await act(async () => {
      await result.current.handlePublishClick();
    });

    expect(result.current.mrUrl).toBeNull();
    expect(result.current.mrState).toBe('none');
    expect(reload).toHaveBeenCalled();
  });
});
