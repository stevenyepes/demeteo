// Unit tests for the `SET_LIVENESS` reducer case in
// `src/context/ProjectContext.tsx`. Pure reducer, no DOM.
//
// Pins down the contract other subtasks (the liveness hook, the dot
// component, ProjectRail) build against without needing to coordinate
// live:
//
//   - dispatching SET_LIVENESS updates only the matching project's
//     `liveness` (and `livenessCheckedAt`, when provided)
//   - all other projects in the array are referentially unchanged
//   - omitting `checkedAt` leaves `livenessCheckedAt` untouched
//
// The reducer itself isn't exported, so these tests exercise it through
// the public `ProjectProvider`/`useProject` surface via a tiny renderer
// harness (mirrors `src/context/NavigationContext.test.tsx`'s direct-call
// style, adapted since this reducer is private to the module).
//
// Runner: `tsc --noEmit`. Assertions throw on failure.

import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { useEffect } from 'react';

import { ProjectProvider, useProject } from './ProjectContext';
import type { Project } from '../types';

function makeProject(id: string, overrides: Partial<Project> = {}): Project {
  return {
    id,
    name: id,
    status: 'idle',
    repos: 0,
    nodes: 0,
    spend: 0,
    tokens: 0,
    ...overrides,
  };
}

type HookHolder = { current: ReturnType<typeof useProject> | null };

async function run() {
  const holder: HookHolder = { current: null };

  function Harness() {
    const ctx = useProject();
    useEffect(() => {
      holder.current = ctx;
    });
    return null;
  }

  let renderer: ReactTestRenderer;
  act(() => {
    renderer = create(
      <ProjectProvider>
        <Harness />
      </ProjectProvider>,
    );
  });

  if (!holder.current) throw new Error('setup: useProject did not populate harness holder');

  const projectA = makeProject('a');
  const projectB = makeProject('b');

  act(() => {
    holder.current!.dispatch({
      type: 'LOAD_PROJECTS',
      projects: [projectA, projectB],
      reposByProject: {},
    });
  });

  // ── (1) SET_LIVENESS updates only the matching project ──
  act(() => {
    holder.current!.dispatch({ type: 'SET_LIVENESS', id: 'a', liveness: 'checking' });
  });
  {
    const [a, b] = holder.current!.state.projects;
    if (a.liveness !== 'checking') {
      throw new Error(`expected project a liveness === 'checking', got '${a.liveness}'`);
    }
    if (b !== projectB) {
      throw new Error('SET_LIVENESS must leave the non-matching project referentially unchanged');
    }
  }

  // ── (2) checkedAt is set when provided ──
  act(() => {
    holder.current!.dispatch({
      type: 'SET_LIVENESS',
      id: 'a',
      liveness: 'online',
      checkedAt: '2026-07-11T00:00:00Z',
    });
  });
  {
    const [a] = holder.current!.state.projects;
    if (a.liveness !== 'online') {
      throw new Error(`expected project a liveness === 'online', got '${a.liveness}'`);
    }
    if (a.livenessCheckedAt !== '2026-07-11T00:00:00Z') {
      throw new Error(`expected project a livenessCheckedAt to be set, got '${a.livenessCheckedAt}'`);
    }
  }

  // ── (3) omitting checkedAt leaves livenessCheckedAt untouched ──
  act(() => {
    holder.current!.dispatch({ type: 'SET_LIVENESS', id: 'a', liveness: 'checking' });
  });
  {
    const [a] = holder.current!.state.projects;
    if (a.liveness !== 'checking') {
      throw new Error(`expected project a liveness === 'checking', got '${a.liveness}'`);
    }
    if (a.livenessCheckedAt !== '2026-07-11T00:00:00Z') {
      throw new Error(
        `expected livenessCheckedAt to survive an update that omits checkedAt, got '${a.livenessCheckedAt}'`,
      );
    }
  }

  // ── (4) checking -> offline transition (the other resolution
  //        ProjectRail's auto-check effect dispatches when a probe
  //        resolves/rejects) ──
  act(() => {
    holder.current!.dispatch({
      type: 'SET_LIVENESS',
      id: 'a',
      liveness: 'offline',
      checkedAt: '2026-07-12T00:00:00Z',
    });
  });
  {
    const [a, b] = holder.current!.state.projects;
    if (a.liveness !== 'offline') {
      throw new Error(`expected project a liveness === 'offline', got '${a.liveness}'`);
    }
    if (a.livenessCheckedAt !== '2026-07-12T00:00:00Z') {
      throw new Error(`expected project a livenessCheckedAt to be updated, got '${a.livenessCheckedAt}'`);
    }
    if (b !== projectB) {
      throw new Error('SET_LIVENESS must leave the non-matching project referentially unchanged');
    }
  }

  act(() => {
    renderer.unmount();
  });
}

export const projectContextLivenessTestResults = run();
