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

const launched = { id: 'f-1', project_id: 'proj-1', title: 'Ship it', status: 'bootstrapping' };

const transports: { transport: string; launch: Partial<LaunchRunParams> }[] = [
  { transport: 'local', launch: { machineId: undefined } },
  { transport: 'detached', launch: { machineId: 'm-1' } },
];

/** What reached Rust: round-tripped through JSON, so an unstated optional key
 *  must be absent, not `null`. */
function launchPayloads(): unknown[] {
  return vi
    .mocked(invoke)
    .mock.calls.filter(([cmd]) => cmd === 'launch_run')
    .map(([, payload]) => JSON.parse(JSON.stringify(payload)));
}

function invokedCommands(): string[] {
  return vi.mocked(invoke).mock.calls.map(([cmd]) => cmd);
}

describe('useLaunchRun', () => {
  it.each(transports)('shows a $transport launch in the rail before its first status event', async ({ launch }) => {
    backend({
      launch_run: () => launched,
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

  it('returns the Feature `launch_run` answers with, unaltered, and lands on it', async () => {
    const shadow = { ...launched, status: 'pending', total_cost: 0, duration: '0s', created_at: 7 };
    backend({ launch_run: () => shadow, feature_status_rollup: () => [] });
    const { result } = renderHook(
      () => ({ launchRun: useLaunchRun({ projectId: 'proj-1' }), navigation: useNavigation() }),
      { wrapper: Providers },
    );

    await act(async () => {
      expect(await result.current.launchRun({ ...params, machineId: 'm-1' })).toEqual(shadow);
    });

    expect(result.current.navigation.view).toMatchObject({ kind: 'detail', featureId: 'f-1' });
  });

  it('sends a local launch as the single `launch_run` args object, with no detached-only key set', async () => {
    backend({ launch_run: () => launched, feature_status_rollup: () => [] });
    const { result } = renderHook(() => useLaunchRun({ projectId: 'proj-1' }), { wrapper: Providers });

    await act(async () => {
      await result.current(params);
    });

    expect(launchPayloads()).toEqual([
      {
        args: {
          machineId: null,
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
          targetRepoId: null,
          maxCostUsd: null,
          maxWallClockSecs: null,
        },
      },
    ]);
    expect(invokedCommands().filter((cmd) => cmd !== 'feature_status_rollup')).toEqual(['launch_run']);
  });

  it('sends a detached launch through the same command, mapping its first repo and minutes', async () => {
    backend({ launch_run: () => launched, feature_status_rollup: () => [] });
    const { result } = renderHook(() => useLaunchRun({ projectId: 'proj-1' }), { wrapper: Providers });

    await act(async () => {
      await result.current({
        ...params,
        machineId: 'm-1',
        targetRepos: ['repo-1', 'repo-2'],
        maxCostUsd: 5,
        maxWallClockMins: 10,
      });
    });

    // `unattended` is absent: Rust defaults a detached run to unattended.
    expect(launchPayloads()).toEqual([
      {
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
          maxCostUsd: 5,
          maxWallClockSecs: 600,
        },
      },
    ]);
    expect(invokedCommands().filter((cmd) => cmd !== 'feature_status_rollup')).toEqual(['launch_run']);
  });

  it('passes an explicit `unattended` through untouched', async () => {
    backend({ launch_run: () => launched, feature_status_rollup: () => [] });
    const { result } = renderHook(() => useLaunchRun({ projectId: 'proj-1' }), { wrapper: Providers });

    await act(async () => {
      await result.current({ ...params, machineId: 'm-1', unattended: true });
    });

    expect(launchPayloads()).toMatchObject([{ args: { machineId: 'm-1', unattended: true } }]);
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
        if (cmd === 'launch_run') return Promise.reject(refusal);
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
        cmd === 'launch_run'
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
        cmd === 'launch_run'
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

  describe('a run the runner accepted but left degraded', () => {
    function render() {
      return renderHook(
        () => ({ launchRun: useLaunchRun({ projectId: 'proj-1' }), navigation: useNavigation(), bus: useErrorBus() }),
        { wrapper: Providers },
      );
    }

    it('still lands on the Feature and leaves a sticky notice that offers Runs', async () => {
      const parked = { ...launched, status: 'pending', credentials_parked: 'unscripted rpc inject_credentials' };
      backend({ launch_run: () => parked, feature_status_rollup: () => [] });
      const { result } = render();

      await act(async () => {
        expect(await result.current.launchRun({ ...params, machineId: 'm-1' })).not.toBeNull();
      });

      expect(result.current.navigation.view).toMatchObject({ kind: 'detail', featureId: 'f-1' });
      expect(result.current.bus.toasts).toHaveLength(1);
      const toast = result.current.bus.toasts[0];
      expect(toast.message).toContain('unscripted rpc inject_credentials');
      expect(toast.message).toContain('Needs credentials');
      expect(toast.dismissable).toBe(false);
      expect(toast.action?.label).toBe('Open runs');

      act(() => toast.action?.onClick());

      expect(result.current.navigation.view).toEqual({ kind: 'remote-inbox' });
      act(() => result.current.bus.clear());
    });

    it('says an unrecorded run will not be reported on', async () => {
      const unrecorded = { ...launched, mirror_unrecorded: 'database is locked during upsert_submitted' };
      backend({ launch_run: () => unrecorded, feature_status_rollup: () => [] });
      const { result } = render();

      await act(async () => {
        expect(await result.current.launchRun({ ...params, machineId: 'm-1' })).not.toBeNull();
      });

      expect(result.current.navigation.view).toMatchObject({ kind: 'detail', featureId: 'f-1' });
      expect(result.current.bus.toasts).toHaveLength(1);
      expect(result.current.bus.toasts[0]).toMatchObject({ kind: 'database' });
      expect(result.current.bus.toasts[0].message).toContain('database is locked during upsert_submitted');
      act(() => result.current.bus.clear());
    });

    it('warns against a second start when the ticket could not record its run', async () => {
      const unrecorded = { ...launched, ticket_unrecorded: 'ticket #3 started Feature f-1, but the disk is full' };
      backend({ launch_run: () => unrecorded, feature_status_rollup: () => [] });
      const { result } = render();

      await act(async () => {
        expect(await result.current.launchRun(params)).not.toBeNull();
      });

      expect(result.current.navigation.view).toMatchObject({ kind: 'detail', featureId: 'f-1' });
      expect(result.current.bus.toasts).toHaveLength(1);
      const toast = result.current.bus.toasts[0];
      expect(toast).toMatchObject({ kind: 'database', dismissable: false });
      expect(toast.message).toContain('ticket #3 started Feature f-1');
      expect(toast.message).toContain('Do not start the ticket again');
      act(() => result.current.bus.clear());
    });

    it.each(transports)('posts nothing for a clean $transport launch', async ({ launch }) => {
      backend({ launch_run: () => launched, feature_status_rollup: () => [] });
      const { result } = render();

      await act(async () => {
        expect(await result.current.launchRun({ ...params, ...launch })).not.toBeNull();
      });

      expect(result.current.navigation.view).toMatchObject({ kind: 'detail', featureId: 'f-1' });
      expect(result.current.bus.toasts).toEqual([]);
    });
  });
});
