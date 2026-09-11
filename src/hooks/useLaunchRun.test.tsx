import { invoke } from '@tauri-apps/api/core';
import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { NavigationProvider, ProjectProvider, useProject } from '../context';
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
    <NavigationProvider>
      <ProjectProvider>{children}</ProjectProvider>
    </NavigationProvider>
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
});
