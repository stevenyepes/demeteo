import { invoke } from '@tauri-apps/api/core';
import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { ProjectProvider, projectReducer, useProject, type ProjectState } from './ProjectContext';
import type { ProjectActivityMap } from '../lib/projectActivity';
import type { Project } from '../types';

function project(id: string): Project {
  return { id, name: `p-${id}`, status: 'idle', repos: 1, nodes: 0, spend: 0, tokens: 0 };
}

const activity: ProjectActivityMap = {
  'proj-1': { active: 2, needsYou: 0 },
  'proj-2': { active: 0, needsYou: 1 },
};

function stateWith(activityByProject: ProjectActivityMap): ProjectState {
  return {
    projects: [project('proj-1'), project('proj-2')],
    currentProjectId: 'proj-1',
    providers: [],
    reposByProject: {},
    activityByProject,
    initialLoadError: '',
  };
}

const seeded = stateWith(activity);

describe('SET_PROJECT_ACTIVITY', () => {
  it('is the map the rail reads', () => {
    const after = projectReducer(stateWith({}), { type: 'SET_PROJECT_ACTIVITY', activity });

    expect(after.activityByProject).toBe(activity);
  });

  it('leaves the projects and the current selection alone', () => {
    const after = projectReducer(seeded, { type: 'SET_PROJECT_ACTIVITY', activity: {} });

    expect(after.projects).toBe(seeded.projects);
    expect(after.currentProjectId).toBe('proj-1');
  });

  it('returns the same state when the fold handed back the map already held', () => {
    const after = projectReducer(seeded, { type: 'SET_PROJECT_ACTIVITY', activity });

    expect(after).toBe(seeded);
  });

  it('returns a new state when a count moved', () => {
    const moved: ProjectActivityMap = {
      'proj-1': { active: 3, needsYou: 0 },
      'proj-2': { active: 0, needsYou: 1 },
    };

    const after = projectReducer(seeded, { type: 'SET_PROJECT_ACTIVITY', activity: moved });

    expect(after).not.toBe(seeded);
    expect(after.activityByProject).toBe(moved);
  });
});

describe('the actions that must not touch activityByProject', () => {
  it('UPDATE_PROJECTS leaves it reference-equal', () => {
    const after = projectReducer(seeded, {
      type: 'UPDATE_PROJECTS',
      updater: (prev) => prev.map((p) => ({ ...p, name: 'renamed', nodes: 8 })),
    });

    expect(after.projects[0].name).toBe('renamed');
    expect(after.activityByProject).toBe(activity);
    expect(after.activityByProject).toEqual({
      'proj-1': { active: 2, needsYou: 0 },
      'proj-2': { active: 0, needsYou: 1 },
    });
  });

  it('LOAD_PROJECTS leaves it reference-equal', () => {
    const after = projectReducer(seeded, {
      type: 'LOAD_PROJECTS',
      projects: [project('proj-3')],
      reposByProject: {},
    });

    expect(after.activityByProject).toBe(activity);
  });

  it('ADD_PROJECT and SET_PROVIDERS leave it reference-equal', () => {
    const added = projectReducer(seeded, { type: 'ADD_PROJECT', project: project('proj-3') });
    expect(added.activityByProject).toBe(activity);

    const providers = projectReducer(seeded, { type: 'SET_PROVIDERS', providers: [] });
    expect(providers.activityByProject).toBe(activity);
  });
});

describe('REMOVE_PROJECT', () => {
  it('drops the removed id and rewrites no other entry', () => {
    const after = projectReducer(seeded, { type: 'REMOVE_PROJECT', id: 'proj-1' });

    expect(after.activityByProject).toEqual({ 'proj-2': { active: 0, needsYou: 1 } });
    expect(after.activityByProject['proj-2']).toBe(activity['proj-2']);
  });
});

describe('the initial slice', () => {
  it('starts empty rather than undefined, so a lookup miss is a miss', () => {
    const empty = projectReducer(stateWith({}), { type: 'SET_ERROR', error: 'boom' });

    expect(empty.activityByProject).toEqual({});
  });
});

describe('refreshProjectActivity', () => {
  it('reads the rollup into the map the rail reads', async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) =>
      cmd === 'feature_status_rollup'
        ? Promise.resolve([{ project_id: 'proj-1', status: 'awaiting_gate', count: 1 }])
        : Promise.reject(new Error(`unexpected command ${cmd}`)),
    );
    const { result } = renderHook(() => useProject(), { wrapper: ProjectProvider });

    act(() => result.current.refreshProjectActivity());

    await waitFor(() =>
      expect(result.current.state.activityByProject).toEqual({ 'proj-1': { active: 0, needsYou: 1 } }),
    );
  });
});
