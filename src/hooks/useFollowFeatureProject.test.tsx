import { invoke } from '@tauri-apps/api/core';
import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { NavigationProvider, ProjectProvider, useNavigation, useProject } from '../context';
import type { Project } from '../types';
import { useFollowFeatureProject } from './useFollowFeatureProject';

/** Rejects anything it was not told to answer. */
function backend(answers: Record<string, (args: Record<string, unknown>) => unknown>): void {
  vi.mocked(invoke).mockImplementation((cmd: string, args?: unknown) =>
    cmd in answers
      ? Promise.resolve(answers[cmd]((args ?? {}) as Record<string, unknown>))
      : Promise.reject(new Error(`unexpected command ${cmd}`)),
  );
}

function Providers({ children }: { children: React.ReactNode }) {
  return (
    <NavigationProvider>
      <ProjectProvider>{children}</ProjectProvider>
    </NavigationProvider>
  );
}

const project = (id: string): Project => ({ id, name: id, status: 'idle', repos: 0, nodes: 0, spend: 0, tokens: 0 });

const OWNER: Record<string, string> = { 'f-leteo': 'p-leteo', 'f-orphan': 'p-gone' };

function setup() {
  backend({
    feature_get: ({ featureId }) => ({ id: featureId, project_id: OWNER[featureId as string], title: 't', status: 'running' }),
  });
  const { result } = renderHook(
    () => {
      useFollowFeatureProject();
      return { nav: useNavigation(), project: useProject() };
    },
    { wrapper: Providers },
  );
  act(() => {
    result.current.project.dispatch({ type: 'LOAD_PROJECTS', projects: [project('p-demeteo'), project('p-leteo')], reposByProject: {} });
    result.current.project.dispatch({ type: 'SET_CURRENT', id: 'p-demeteo' });
  });
  return result;
}

describe('useFollowFeatureProject', () => {
  afterEach(() => vi.mocked(invoke).mockReset());

  it("selects the project of a feature opened from another workspace's gate", async () => {
    const result = setup();
    act(() => {
      result.current.nav.navigate(
        { kind: 'detail', featureId: 'f-leteo', featureTitle: 'Feature Pipeline', gateStepExecutionId: 'se-1' },
        'replace',
      );
    });
    await waitFor(() => expect(result.current.project.state.currentProjectId).toBe('p-leteo'));
  });

  it('keeps the current project when the feature belongs to none the rail holds', async () => {
    const result = setup();
    act(() => {
      result.current.nav.navigate({ kind: 'detail', featureId: 'f-orphan', featureTitle: 'x' });
    });
    await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalledWith('feature_get', { featureId: 'f-orphan' }));
    await act(async () => {});
    expect(result.current.project.state.currentProjectId).toBe('p-demeteo');
  });
});
