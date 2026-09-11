import { invoke } from '@tauri-apps/api/core';
import { render, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { NavigationProvider, ProjectProvider, useProject } from '../context';
import type { ProjectActivityMap } from '../lib/projectActivity';
import RemoteRunInbox from './RemoteRunInbox';

/** Rejects anything it was not told to answer, so an inbox that skipped the
 *  rollup cannot pass against a default. */
function backend(answers: Record<string, () => unknown>): void {
  vi.mocked(invoke).mockImplementation((cmd: string) =>
    cmd in answers ? Promise.resolve(answers[cmd]()) : Promise.reject(new Error(`unexpected command ${cmd}`)),
  );
}

describe('RemoteRunInbox', () => {
  it('re-reads the rail after reconcile hydrates the shadow rows', async () => {
    backend({
      remote_list_mirrored_runs: () => [],
      get_machines: () => [],
      remote_reconcile_runs: () => [],
      feature_status_rollup: () => [{ project_id: 'proj-1', status: 'awaiting_gate', count: 1 }],
    });
    let activity: ProjectActivityMap = {};
    function RailProbe() {
      activity = useProject().state.activityByProject;
      return null;
    }

    render(
      <NavigationProvider>
        <ProjectProvider>
          <RemoteRunInbox />
          <RailProbe />
        </ProjectProvider>
      </NavigationProvider>,
    );

    await waitFor(() => expect(activity).toEqual({ 'proj-1': { active: 0, needsYou: 1 } }));
  });
});
