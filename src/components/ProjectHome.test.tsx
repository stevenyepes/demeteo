import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect, useState, type ReactElement, type ReactNode } from 'react';
import { describe, expect, it, beforeEach, vi } from 'vitest';
import { invoke, type InvokeArgs } from '@tauri-apps/api/core';

import {
  NavigationProvider,
  ProjectProvider,
  UIStateProvider,
  TerminalPanelProvider,
  useProject,
} from '../context';
import ProjectHome from './ProjectHome';
import type { Project } from '../types';

// ProjectHome resolves its `activeProject` synchronously on first render
// (`projects.find(...)!`), so it can't be seeded via a sibling's effect that
// fires *after* ProjectHome has already mounted once with an empty project
// list — the non-null assertion would already have thrown. `ProjectSeed`
// dispatches into the real `ProjectProvider` from an effect and withholds
// `children` until that dispatch has landed, so `ProjectHome` only ever
// mounts once the project is already present in context — no hook mocking
// required.
function ProjectSeed({ project, children }: { project: Project; children: ReactNode }): ReactElement | null {
  const { dispatch } = useProject();
  const [seeded, setSeeded] = useState(false);
  useEffect(() => {
    dispatch({ type: 'LOAD_PROJECTS', projects: [project], reposByProject: {} });
    dispatch({ type: 'SET_CURRENT', id: project.id });
    setSeeded(true);
    // Seed once per mount — a fresh `mount()` call is used for each test.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  if (!seeded) return null;
  return <>{children}</>;
}

function baseProject(overrides: Partial<Project> = {}): Project {
  return {
    id: 'proj-1',
    name: 'Demo Project',
    status: 'idle',
    repos: 1,
    nodes: 0,
    spend: 0,
    tokens: 0,
    ...overrides,
  };
}

/**
 * Backend stub covering every command `ProjectHome`'s `fetchWorkspaceData`
 * effect fires on mount, plus the terminal-session path `StartSessionButton`
 * drives through the real `TerminalPanelProvider`. `resolve_repo_dir` echoes
 * back a path derived from the requested `repoPath` so a test can assert
 * which repo a click actually resolved against.
 */
function mockBackend(repoPaths: string[]) {
  vi.mocked(invoke).mockImplementation((cmd: string, args?: InvokeArgs) => {
    switch (cmd) {
      case 'fetch_active_features':
        return Promise.resolve([]);
      case 'get_repositories_for_project':
        return Promise.resolve(repoPaths.map((p) => ({ repo_path: p })));
      case 'workflow_list':
        return Promise.resolve([]);
      case 'remote_list_mirrored_runs':
        return Promise.resolve([]);
      case 'list_terminal_sessions':
        return Promise.resolve([]);
      case 'resolve_repo_dir': {
        const { repoPath } = (args ?? {}) as { repoPath?: string };
        return Promise.resolve(`/resolved${repoPath}`);
      }
      case 'start_terminal_session':
        return Promise.resolve({ session_id: 'sess_1', launch_command: null });
      default:
        return Promise.resolve(undefined);
    }
  });
}

function commandsOf(name: string): Array<Record<string, unknown> | undefined> {
  return vi
    .mocked(invoke)
    .mock.calls.filter(([c]) => c === name)
    .map(([, a]) => a as Record<string, unknown> | undefined);
}

function mount(project: Project) {
  render(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <TerminalPanelProvider>
            <ProjectSeed project={project}>
              <ProjectHome />
            </ProjectSeed>
          </TerminalPanelProvider>
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );
}

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

describe('ProjectHome — persistent Start Session affordance', () => {
  it('renders the StartSessionButton for a local project', async () => {
    mockBackend(['/repo/one']);
    mount(baseProject({ compute_type: 'local' }));

    await waitFor(() => expect(screen.getByTestId('start-session-button')).toBeInTheDocument());
    // No repo selector for a single-repo project.
    expect(screen.queryByText('Repository:')).not.toBeInTheDocument();
  });

  it('renders the StartSessionButton for a remote project on the default (pipelines) tab', async () => {
    mockBackend(['/repo/one']);
    mount(baseProject({ compute_type: 'remote', remote_host: 'gpu-box' }));

    await waitFor(() => expect(screen.getByTestId('start-session-button')).toBeInTheDocument());
    expect(screen.getByTestId('start-session-primary')).toBeInTheDocument();
  });

  it('keeps the StartSessionButton visible after switching a remote project to the Terminal tab', async () => {
    mockBackend(['/repo/one']);
    mount(baseProject({ compute_type: 'remote', remote_host: 'gpu-box' }));

    await waitFor(() => expect(screen.getByTestId('start-session-button')).toBeInTheDocument());
    await act(async () => {
      await userEvent.click(screen.getByText('Terminal').closest('button')!);
    });

    await waitFor(() => expect(screen.getByText('Opening the Terminals view…')).toBeInTheDocument());
    expect(screen.getByTestId('start-session-button')).toBeInTheDocument();
  });

  it('disables the button for a project with no repositories rather than opening an unscoped shell', async () => {
    mockBackend([]);
    mount(baseProject({ compute_type: 'local' }));

    await waitFor(() => expect(screen.getByTestId('start-session-button')).toBeInTheDocument());
    expect(screen.getByTestId('start-session-primary')).toBeDisabled();

    await act(async () => {
      screen.getByTestId('start-session-primary').click();
    });
    expect(commandsOf('start_terminal_session')).toHaveLength(0);
  });

  it('leaves TerminalTabOpener auto-open (dedup, no forceNew) intact while the button stacks sessions', async () => {
    mockBackend(['/repo/one']);
    mount(baseProject({ id: 'proj-remote', compute_type: 'remote', remote_host: 'gpu-box' }));

    await waitFor(() => expect(screen.getByTestId('start-session-button')).toBeInTheDocument());
    expect(commandsOf('start_terminal_session')).toHaveLength(0);

    // Switching to the Terminal tab auto-opens exactly one session…
    await act(async () => {
      await userEvent.click(screen.getByText('Terminal').closest('button')!);
    });
    await waitFor(() => expect(commandsOf('start_terminal_session')).toHaveLength(1));

    // …and re-entering the tab reuses that tab via `logicalTabKey` rather than
    // starting a second session — i.e. the opener still passes no `forceNew`.
    await act(async () => {
      await userEvent.click(screen.getByText('Pipelines').closest('button')!);
    });
    await act(async () => {
      await userEvent.click(screen.getByText('Terminal').closest('button')!);
    });
    await waitFor(() => expect(screen.getByText('Opening the Terminals view…')).toBeInTheDocument());
    expect(commandsOf('start_terminal_session')).toHaveLength(1);

    // The hero button is independent: each click stacks another session.
    await act(async () => {
      await userEvent.click(screen.getByTestId('start-session-primary'));
    });
    await waitFor(() => expect(commandsOf('start_terminal_session')).toHaveLength(2));
    await act(async () => {
      await userEvent.click(screen.getByTestId('start-session-primary'));
    });
    await waitFor(() => expect(commandsOf('start_terminal_session')).toHaveLength(3));
  });

  it('shows a repo selector next to the button for a multi-repo local project (previously unreachable)', async () => {
    mockBackend(['/repo/one', '/repo/two']);
    mount(baseProject({ compute_type: 'local' }));

    await waitFor(() => expect(screen.getByTestId('start-session-button')).toBeInTheDocument());
    expect(screen.getByText('Repository:')).toBeInTheDocument();
    expect(screen.getByText('/repo/one')).toBeInTheDocument();
    expect(screen.getByText('/repo/two')).toBeInTheDocument();
  });

  it('shows exactly one (not duplicated) repo selector for a multi-repo remote project on the Terminal tab', async () => {
    mockBackend(['/repo/one', '/repo/two']);
    mount(baseProject({ compute_type: 'remote', remote_host: 'gpu-box' }));

    await waitFor(() => expect(screen.getByTestId('start-session-button')).toBeInTheDocument());
    await act(async () => {
      await userEvent.click(screen.getByText('Terminal').closest('button')!);
    });

    await waitFor(() => expect(screen.getByText('Opening the Terminals view…')).toBeInTheDocument());
    expect(screen.getAllByText('Repository:')).toHaveLength(1);
  });

  it('starts a terminal session scoped to the resolved repo path for a local project', async () => {
    mockBackend(['/repo/one']);
    mount(baseProject({ id: 'proj-local', compute_type: 'local' }));

    await waitFor(() => expect(screen.getByTestId('start-session-button')).toBeInTheDocument());

    await act(async () => {
      await userEvent.click(screen.getByTestId('start-session-primary'));
    });

    await waitFor(() => expect(commandsOf('start_terminal_session')).toHaveLength(1));

    expect(commandsOf('resolve_repo_dir')[0]).toMatchObject({
      projectId: 'proj-local',
      repoPath: '/repo/one',
    });
    expect(commandsOf('start_terminal_session')[0]).toMatchObject({
      machineId: 'local',
      workDir: '/resolved/repo/one',
    });
  });

  it('starts a terminal session scoped to the remote host and repo path for a remote project', async () => {
    mockBackend(['/repo/remote']);
    mount(baseProject({ id: 'proj-remote', compute_type: 'remote', remote_host: 'gpu-box' }));

    await waitFor(() => expect(screen.getByTestId('start-session-button')).toBeInTheDocument());

    await act(async () => {
      await userEvent.click(screen.getByTestId('start-session-primary'));
    });

    await waitFor(() => expect(commandsOf('start_terminal_session')).toHaveLength(1));

    expect(commandsOf('resolve_repo_dir')[0]).toMatchObject({
      projectId: 'proj-remote',
      repoPath: '/repo/remote',
    });
    expect(commandsOf('start_terminal_session')[0]).toMatchObject({
      machineId: 'gpu-box',
      workDir: '/resolved/repo/remote',
    });
  });

  it('threads the repo selected via the repo selector through to the next session start', async () => {
    mockBackend(['/repo/one', '/repo/two']);
    mount(baseProject({ id: 'proj-multi', compute_type: 'local' }));

    await waitFor(() => expect(screen.getByTestId('start-session-button')).toBeInTheDocument());
    // Defaults to the first repo returned by the backend.
    const select = screen.getByText('Repository:').closest('div')!.querySelector('select')!;
    expect(select.value).toBe('/repo/one');

    await act(async () => {
      await userEvent.selectOptions(select, '/repo/two');
    });
    expect(select.value).toBe('/repo/two');

    await act(async () => {
      await userEvent.click(screen.getByTestId('start-session-primary'));
    });

    await waitFor(() => expect(commandsOf('start_terminal_session')).toHaveLength(1));

    expect(commandsOf('resolve_repo_dir')[0]).toMatchObject({
      projectId: 'proj-multi',
      repoPath: '/repo/two',
    });
    expect(commandsOf('start_terminal_session')[0]).toMatchObject({
      machineId: 'local',
      workDir: '/resolved/repo/two',
    });
  });
});
