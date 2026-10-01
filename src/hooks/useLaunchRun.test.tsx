import { invoke } from '@tauri-apps/api/core';
import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { NavigationProvider, ProjectProvider, UIStateProvider, useNavigation, useProject, useUIState } from '../context';
import { ErrorBusProvider, useErrorBus } from '../lib/errorBus';
import {
  cachedRunnerCompatibility,
  fetchRunnerCompatibility,
  invalidateRunnerCompatibility,
} from '../lib/runnerCompatibility';
import type { RunnerCompatibility } from '../types';
import { useLaunchRun, type LaunchRunParams } from './useLaunchRun';

/** Rejects anything it was not told to answer, so a launch that skipped the
 *  rollup cannot pass against a default. */
function backend(answers: Record<string, () => unknown>): void {
  vi.mocked(invoke).mockImplementation((cmd: string) =>
    cmd in answers ? Promise.resolve(answers[cmd]()) : Promise.reject(new Error(`unexpected command ${cmd}`)),
  );
}

function Providers({ children }: { children: React.ReactNode }) {
  return (
    <ErrorBusProvider>
      <NavigationProvider>
        <ProjectProvider>
          <UIStateProvider>{children}</UIStateProvider>
        </ProjectProvider>
      </NavigationProvider>
    </ErrorBusProvider>
  );
}

const params: LaunchRunParams = { workflowId: 'wf-1', title: 'Ship it', description: '' };

const transports: { transport: string; launch: Partial<LaunchRunParams>; answers: Record<string, () => unknown> }[] = [
  {
    transport: 'local',
    launch: { machineId: undefined },
    answers: {
      start_feature: () => ({ id: 'f-1', project_id: 'proj-1', title: 'Ship it', status: 'bootstrapping' }),
    },
  },
  {
    transport: 'detached',
    launch: { machineId: 'm-1' },
    answers: {
      remote_submit_run: () => ({ run_id: 'r-1', machine_id: 'm-1', status: 'pending', feature_id: 'f-1' }),
    },
  },
];

describe('useLaunchRun', () => {
  it.each(transports)('shows a $transport launch in the rail before its first status event', async ({ launch, answers }) => {
    backend({
      ...answers,
      feature_status_rollup: () => [{ project_id: 'proj-1', status: 'bootstrapping', count: 1 }],
    });
    const { result } = renderHook(
      () => ({ launchRun: useLaunchRun({ projectId: 'proj-1' }), project: useProject() }),
      { wrapper: Providers },
    );

    await act(async () => {
      expect(await result.current.launchRun({ ...params, ...launch })).not.toBeNull();
    });

    await waitFor(() =>
      expect(result.current.project.state.activityByProject).toEqual({ 'proj-1': { active: 1, needsYou: 0 } }),
    );
  });

  it('submits a detached launch as the single `args` object the Rust command deserializes', async () => {
    backend({
      remote_submit_run: () => ({ run_id: 'r-1', machine_id: 'm-1', status: 'pending', feature_id: 'f-1' }),
      feature_status_rollup: () => [],
    });
    const { result } = renderHook(() => useLaunchRun({ projectId: 'proj-1' }), { wrapper: Providers });

    await act(async () => {
      await result.current({ ...params, machineId: 'm-1', targetRepos: ['repo-1'], maxWallClockMins: 10 });
    });

    const submit = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === 'remote_submit_run');
    // Round-tripped through JSON because that is what reaches Rust: an
    // unstated `origin`/`diffBaseBranch` must be absent, not `null`.
    expect(JSON.parse(JSON.stringify(submit?.[1]))).toEqual({
      args: {
        machineId: 'm-1',
        projectId: 'proj-1',
        workflowId: 'wf-1',
        title: 'Ship it',
        description: '',
        agentKind: null,
        model: null,
        effort: null,
        commitArtifacts: null,
        loopIterations: null,
        maxBudgetUsd: null,
        stepOverrides: null,
        stagedAttachments: [],
        targetRepoId: 'repo-1',
        unattended: false,
        maxCostUsd: null,
        maxWallClockSecs: 600,
      },
    });
  });

  describe('a refused detached submit', () => {
    const behind: RunnerCompatibility = {
      verdict: 'runner_behind',
      runner: '1.1.0',
      runner_channel: 'stable',
      app: '1.2.0',
      app_channel: 'stable',
    };
    const message = 'demeteo-runner 1.1.0 (stable) on box is older than Demeteo 1.2.0 (stable) — upgrade the runner.';

    function render() {
      return renderHook(
        () => ({
          launchRun: useLaunchRun({ projectId: 'proj-1' }),
          navigation: useNavigation(),
          ui: useUIState(),
          bus: useErrorBus(),
        }),
        { wrapper: Providers },
      );
    }

    afterEach(() => {
      invalidateRunnerCompatibility();
    });

    it('offers machine settings for a runner_incompatible refusal and drops the stale verdict', async () => {
      const refusal = { kind: 'runner_incompatible', message, compatibility: behind };
      vi.mocked(invoke).mockImplementation((cmd: string) => {
        if (cmd === 'remote_runner_compatibility') {
          return Promise.resolve({ verdict: 'compatible', version: '1.1.0', channel: 'stable', message: 'ok' });
        }
        if (cmd === 'remote_submit_run') return Promise.reject(refusal);
        return Promise.reject(new Error(`unexpected command ${cmd}`));
      });
      await fetchRunnerCompatibility('m-1');
      expect(cachedRunnerCompatibility('m-1')).not.toBeNull();
      const { result } = render();
      act(() => result.current.ui.uiDispatch({ type: 'OPEN_START_FEATURE' }));
      const onRunnerRefused = vi.fn();

      await act(async () => {
        expect(
          await result.current.launchRun(
            { ...params, description: 'What it does', machineId: 'm-1' },
            { onRunnerRefused },
          ),
        ).toBeNull();
      });

      expect(onRunnerRefused).toHaveBeenCalledOnce();
      expect(cachedRunnerCompatibility('m-1')).toBeNull();
      const toast = result.current.bus.toasts[0];
      expect(toast).toMatchObject({ kind: 'runner_incompatible', message });
      expect(toast.action?.label).toBe('Open machine settings');

      act(() => toast.action?.onClick());

      // The hook only asks the open modal to leave; the modal, absent here,
      // builds the draft from its own fields (`StartFeatureHost.test.tsx`).
      expect(result.current.ui.ui.startFeatureLeaveRequested).toBe(true);
      expect(result.current.ui.ui.startFeatureSeed).toBeNull();
      expect(result.current.navigation.view).toEqual({ kind: 'settings' });
      act(() => result.current.bus.clear());
    });

    it('keeps no draft when the refused launch did not come from the Start Feature modal', async () => {
      vi.mocked(invoke).mockImplementation((cmd: string) =>
        cmd === 'remote_submit_run'
          ? Promise.reject({ kind: 'runner_incompatible', message, compatibility: behind })
          : Promise.reject(new Error(`unexpected command ${cmd}`)),
      );
      const { result } = render();

      await act(async () => {
        expect(await result.current.launchRun({ ...params, machineId: 'm-1' })).toBeNull();
      });
      act(() => result.current.bus.toasts[0].action?.onClick());

      expect(result.current.navigation.view).toEqual({ kind: 'settings' });
      expect(result.current.ui.ui.startFeatureSeed).toBeNull();
      expect(result.current.ui.ui.startFeatureLeaveRequested).toBe(false);
      act(() => result.current.bus.clear());
    });

    it('reports any other refusal through the generic path', async () => {
      vi.mocked(invoke).mockImplementation((cmd: string) =>
        cmd === 'remote_submit_run'
          ? Promise.reject({ kind: 'transport', message: 'ssh: connection refused' })
          : Promise.reject(new Error(`unexpected command ${cmd}`)),
      );
      const { result } = render();
      const onRunnerRefused = vi.fn();

      await act(async () => {
        expect(await result.current.launchRun({ ...params, machineId: 'm-1' }, { onRunnerRefused })).toBeNull();
      });

      expect(onRunnerRefused).not.toHaveBeenCalled();
      const toast = result.current.bus.toasts[0];
      expect(toast).toMatchObject({ kind: 'transport', message: 'ssh: connection refused' });
      expect(toast.action).toBeUndefined();
      expect(result.current.navigation.view).not.toEqual({ kind: 'settings' });
      act(() => result.current.bus.clear());
    });
  });
});
