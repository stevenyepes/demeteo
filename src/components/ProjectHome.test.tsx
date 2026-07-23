// Tests for the inline composer's clipboard image paste handler in
// ProjectHome. Ticket-5 wires a scoped paste target onto the composer
// container so the compact `AttachmentDropzone` (which returns before the
// full dropzone paste target) still receives clipboard files. The handler
// must also refrain from intercepting ordinary text paste in the title
// input.
//
// Also covers the persistent Start Session affordance: StartSessionButton
// renders for local/remote projects, stacks sessions independently of the
// TerminalTabOpener auto-open, and threads the selected repo through to the
// resolved session start.

import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
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

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: vi.fn().mockResolvedValue(() => {}),
  }),
}));

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

/**
 * Render helper for the clipboard-paste suite below. Seeds a single active
 * project with the real backend mock (via `mockBackend`) so the composer's
 * `fetchWorkspaceData` effect resolves without touching Tauri.
 */
function renderHome() {
  mockBackend(['/repo/one']);
  mount(baseProject({ compute_type: 'local' }));
}

interface ClipboardItemFixture {
  kind: string;
  type: string;
  getAsFile: () => File | null;
}

function clipboardData(items: ClipboardItemFixture[]): DataTransfer {
  return { items } as unknown as DataTransfer;
}

function imageItem(file: File): ClipboardItemFixture {
  return { kind: 'file', type: file.type, getAsFile: () => file };
}

function paste(node: Element, items: ClipboardItemFixture[]) {
  const event = new Event('paste', { bubbles: true, cancelable: true });
  Object.defineProperty(event, 'clipboardData', { value: clipboardData(items) });
  const preventDefault = vi.spyOn(event, 'preventDefault');
  fireEvent(node, event);
  return preventDefault;
}

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

describe('ProjectHome inline composer paste', () => {
  it('prevents the event and stages a supported image paste on the composer container', async () => {
    renderHome();
    const composer = await screen.findByTestId('project-home-composer');
    const file = new File(['image bytes'], 'pasted.png', { type: 'image/png' });

    const preventDefault = paste(composer, [imageItem(file)]);

    expect(preventDefault).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(screen.getByText(/pasted\.png/)).toBeInTheDocument(),
    );
  });

  it('stages every supported image from a multi-image paste in clipboard order', async () => {
    renderHome();
    const composer = await screen.findByTestId('project-home-composer');
    const png = new File(['png'], 'alpha.png', { type: 'image/png' });
    const webp = new File(['webp'], 'bravo.webp', { type: 'image/webp' });

    paste(composer, [imageItem(png), imageItem(webp)]);

    await waitFor(() => {
      expect(screen.getByText(/alpha\.png/)).toBeInTheDocument();
      expect(screen.getByText(/bravo\.webp/)).toBeInTheDocument();
    });
  });

  it('does not stage or prevent an unsupported-image-only paste', async () => {
    renderHome();
    const composer = await screen.findByTestId('project-home-composer');
    const bmp = new File(['bmp'], 'clipboard.bmp', { type: 'image/bmp' });

    const preventDefault = paste(composer, [imageItem(bmp)]);

    expect(preventDefault).not.toHaveBeenCalled();
    expect(screen.queryByText(/clipboard\.bmp/)).not.toBeInTheDocument();
  });

  it('keeps normal text paste in the title input intact (no preventDefault)', async () => {
    renderHome();
    const composer = await screen.findByTestId('project-home-composer');
    const input = composer.querySelector('input') as HTMLInputElement;
    expect(input).toBeTruthy();

    // A paste fired on the title input bubbles up to our handler; with
    // `e.target instanceof HTMLInputElement` set, the handler must bail
    // without calling `preventDefault`, leaving the input's native text
    // paste path active.
    const event = new Event('paste', { bubbles: true, cancelable: true });
    const dt = { items: [], getData: () => '' } as unknown as DataTransfer;
    Object.defineProperty(event, 'clipboardData', { value: dt });
    const preventDefault = vi.spyOn(event, 'preventDefault');
    fireEvent(input, event);

    expect(preventDefault).not.toHaveBeenCalled();
    fireEvent.change(input, { target: { value: 'ship clip support' } });
    expect(input.value).toBe('ship clip support');
  });
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
