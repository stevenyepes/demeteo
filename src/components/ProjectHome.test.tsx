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
import { afterEach, describe, expect, it, beforeEach, vi } from 'vitest';
import { invoke, type InvokeArgs } from '@tauri-apps/api/core';
import { UI_PREF_WRITE_DEBOUNCE_MS } from '../lib/uiPrefs';

import {
  NavigationProvider,
  ProjectProvider,
  UIStateProvider,
  TerminalPanelProvider,
  useProject,
  useUIState,
} from '../context';
import ProjectHome from './ProjectHome';
import type { Feature, Project, Provider } from '../types';

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
function mockBackend(repoPaths: string[] | Record<string, string[]>) {
  vi.mocked(invoke).mockImplementation((cmd: string, args?: InvokeArgs) => {
    switch (cmd) {
      case 'fetch_active_features':
        return Promise.resolve([]);
      case 'get_repositories_for_project':
        {
          const { projectId } = (args ?? {}) as { projectId?: string };
          const paths = Array.isArray(repoPaths) ? repoPaths : repoPaths[projectId ?? ''] ?? [];
          return Promise.resolve(paths.map((repo_path, index) => ({
            id: `${projectId ?? 'project'}-repo-${index}`,
            repo_path,
            provider_id: 'provider-1',
          })));
        }
      case 'workflow_list':
        return Promise.resolve([]);
      case 'remote_list_mirrored_runs':
        return Promise.resolve([]);
      case 'list_terminal_sessions':
        return Promise.resolve([]);
      case 'list_terminal_locations':
        return Promise.resolve({ main_branch: 'chore/left-here', worktrees: [] });
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
              <StartFeatureSeedProbe />
            </ProjectSeed>
          </TerminalPanelProvider>
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );
}

function ProjectSwitchSeed({ projects, children }: { projects: Project[]; children: ReactNode }): ReactElement | null {
  const { dispatch } = useProject();
  const [seeded, setSeeded] = useState(false);
  useEffect(() => {
    dispatch({ type: 'LOAD_PROJECTS', projects, reposByProject: {} });
    dispatch({ type: 'SET_CURRENT', id: projects[0].id });
    setSeeded(true);
    // This fixture is deliberately mounted once per test.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  if (!seeded) return null;
  return <>{children}</>;
}

function ProjectSwitcher({ projectId }: { projectId: string }): ReactElement {
  const { dispatch } = useProject();
  return <button type="button" onClick={() => dispatch({ type: 'SET_CURRENT', id: projectId })}>Switch project</button>;
}

function mountSwitchable(projects: Project[]) {
  render(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <TerminalPanelProvider>
            <ProjectSwitchSeed projects={projects}>
              <ProjectSwitcher projectId={projects[1].id} />
              <ProjectHome />
            </ProjectSwitchSeed>
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

function StartFeatureSeedProbe() {
  const { ui } = useUIState();
  return (
    <output data-testid="start-feature-seed">
      {JSON.stringify(ui.startFeatureSeed)}
    </output>
  );
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

/**
 * A drift read that rejected has to reach the row as an unmeasured count. The
 * project view is the surface that shows the whole queue at once, so a feature
 * nobody could count and a feature nobody has counted yet rendering alike is
 * exactly where the three-state signal quietly becomes two.
 */
describe('ProjectHome staleness', () => {
  function featureWithOpenRequest(): Feature {
    return {
      id: 'f-1',
      project_id: 'proj-1',
      title: 'Say how far behind a branch is',
      status: 'completed',
      mr_url: 'https://github.com/stvcloud/demeteo/pull/7',
      mr_state: 'open',
    } as unknown as Feature;
  }

  function mockBackendWithDrift(drift: () => Promise<unknown>) {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'fetch_active_features':
          return Promise.resolve([featureWithOpenRequest()]);
        case 'feature_drift':
          return drift();
        case 'get_repositories_for_project':
        case 'workflow_list':
        case 'remote_list_mirrored_runs':
        case 'list_terminal_sessions':
          return Promise.resolve([]);
        default:
          return Promise.resolve(undefined);
      }
    });
    mount(baseProject({ compute_type: 'local' }));
  }

  it('renders a count that could not be taken as unknown, not as up to date', async () => {
    mockBackendWithDrift(() => Promise.reject('the repository is not on this machine'));

    expect(await screen.findByText('Drift unknown')).toBeInTheDocument();
    expect(screen.queryByText('Up to date')).toBeNull();
  });

  it('renders the count when the read answered', async () => {
    mockBackendWithDrift(() =>
      Promise.resolve({
        divergence: { behind: 4, ahead: 1 },
        base_ref: 'origin/main',
        fetched: false,
        checked_at: 0,
      }),
    );

    expect(await screen.findByText('4 behind')).toBeInTheDocument();
  });
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

  it('recovers a WebKitGTK empty-item image paste through the async clipboard', async () => {
    const clipboardRead = vi.fn().mockResolvedValue([{
      types: ['image/png'],
      getType: vi.fn().mockResolvedValue(new Blob(['png bytes'], { type: 'image/png' })),
    }]);
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { read: clipboardRead } });
    try {
      renderHome();
      const composer = await screen.findByTestId('project-home-composer');

      paste(composer, []);

      await waitFor(() => expect(clipboardRead).toHaveBeenCalledTimes(1));
      expect(await screen.findByText(/pasted-image\.png/)).toBeInTheDocument();
    } finally {
      if (previousClipboard) Object.defineProperty(navigator, 'clipboard', previousClipboard);
      else Reflect.deleteProperty(navigator, 'clipboard');
    }
  });

  it('shows a soft error when a WebKitGTK empty-item image paste is denied', async () => {
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { read: vi.fn().mockRejectedValue(new DOMException('denied', 'NotAllowedError')) },
    });
    try {
      renderHome();
      const composer = await screen.findByTestId('project-home-composer');

      paste(composer, []);

      expect(await screen.findByRole('alert')).toHaveTextContent(/could not read image bytes/i);
    } finally {
      if (previousClipboard) Object.defineProperty(navigator, 'clipboard', previousClipboard);
      else Reflect.deleteProperty(navigator, 'clipboard');
    }
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

  // Two pastes fired before either has staged: both handlers are in flight
  // across the `await`, so a handler that computes the next stage list from the
  // `attachments` it captured at creation time overwrites the first result with
  // the second.
  it('keeps both images when a second paste lands before the first has staged', async () => {
    renderHome();
    const composer = await screen.findByTestId('project-home-composer');

    paste(composer, [imageItem(new File(['first'], 'first.png', { type: 'image/png' }))]);
    paste(composer, [imageItem(new File(['second'], 'second.png', { type: 'image/png' }))]);

    await waitFor(() => {
      expect(screen.getByText(/first\.png/)).toBeInTheDocument();
      expect(screen.getByText(/second\.png/)).toBeInTheDocument();
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

  it('stages a supported image from the focused title input and passes it to the modal seed', async () => {
    renderHome();
    const composer = await screen.findByTestId('project-home-composer');
    const input = composer.querySelector('input') as HTMLInputElement;
    const file = new File(['focused image'], 'focused.png', { type: 'image/png' });

    const preventDefault = paste(input, [imageItem(file)]);

    expect(preventDefault).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.getByText(/focused\.png/)).toBeInTheDocument());
    await userEvent.click(screen.getByRole('button', { name: /continue/i }));

    await waitFor(() => {
      const seed = JSON.parse(screen.getByTestId('start-feature-seed').textContent ?? 'null');
      expect(seed.attachments).toHaveLength(1);
      expect(seed.attachments[0]).toMatchObject({
        name: 'focused.png',
        source_filename: 'focused.png',
        mime: 'image/png',
      });
    });
  });

  it('keeps normal text paste in the focused title input intact (no preventDefault)', async () => {
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
    expect(screen.queryByTestId('project-home-repo-select')).not.toBeInTheDocument();
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
    expect(screen.getByTestId('project-home-repo-select')).toBeInTheDocument();
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
    expect(screen.getAllByTestId('project-home-repo-select')).toHaveLength(1);
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
    const select = screen.getByTestId('project-home-repo-select') as HTMLSelectElement;
    expect(select.value).toBe('proj-multi-repo-0');

    await act(async () => {
      await userEvent.selectOptions(select, 'proj-multi-repo-1');
    });
    expect(select.value).toBe('proj-multi-repo-1');

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

  it('clears the prior project repository before a newly selected project can launch', async () => {
    mockBackend({
      'proj-a': ['/repo/project-a'],
      'proj-b': [],
    });
    mountSwitchable([
      baseProject({ id: 'proj-a', compute_type: 'local' }),
      baseProject({ id: 'proj-b', compute_type: 'local' }),
    ]);

    await waitFor(() => expect(screen.getByTestId('terminal-location-trigger')).not.toBeDisabled());
    await userEvent.click(screen.getByRole('button', { name: 'Switch project' }));

    await waitFor(() => expect(screen.getByTestId('start-session-primary')).toBeDisabled());
    await act(async () => {
      screen.getByTestId('start-session-primary').click();
    });

    expect(commandsOf('resolve_repo_dir')).toHaveLength(0);
    expect(commandsOf('start_terminal_session')).toHaveLength(0);
  });
});

// Regression tests for `handleRetryBootstrap` — reached from the "Retry
// Bootstrap" button on the "Workspace Bootstrap Failed" banner. The bug:
// it re-fetches a stale `get_proposed_strategy` result and a freshly
// re-detected `bootstrap_project` strategy, then overwrites the in-scope
// testCommand (and sibling) state with `ext?.field ?? strategy.field ?? ''`
// — discarding any edit already sitting in the "STRATEGY DETECTED" popup's
// Test Command field from a prior pass through this same handler.
describe('ProjectHome — handleRetryBootstrap testCommand precedence', () => {
  const EXISTING_TEST_COMMAND = 'A: stale existing command';
  const STRATEGY_TEST_COMMAND = 'B: freshly re-detected command';

  function mockRetryBootstrapBackend() {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'fetch_active_features':
          return Promise.resolve([]);
        case 'get_repositories_for_project':
          return Promise.resolve([]);
        case 'workflow_list':
          return Promise.resolve([]);
        case 'remote_list_mirrored_runs':
          return Promise.resolve([]);
        case 'get_proposed_strategy':
          return Promise.resolve({
            worktree_strategy: {
              default_branch: 'main',
              branch_prefix: 'demeteo/features/',
              test_command: EXISTING_TEST_COMMAND,
              pr_template: null,
            },
          });
        case 'bootstrap_project':
          return Promise.resolve({
            default_branch: 'main',
            branch_prefix: 'demeteo/features/',
            test_command: STRATEGY_TEST_COMMAND,
            pr_template: null,
          });
        default:
          return Promise.resolve(undefined);
      }
    });
  }

  async function retryBootstrap() {
    await userEvent.click(await screen.findByRole('button', { name: /Retry Bootstrap/i }));
    await screen.findByText('STRATEGY DETECTED');
  }

  it('shows the value the user just typed, not the stale existing/strategy reads', async () => {
    mockRetryBootstrapBackend();
    mount(baseProject({ status: 'error' }));

    // First pass: open the popup so the Test Command input exists, and
    // simulate a prior user edit in it.
    await retryBootstrap();
    const input = screen.getByPlaceholderText('e.g. npm test or cargo test');
    const typedCommand = 'C: what the user just typed';
    await userEvent.clear(input);
    await userEvent.type(input, typedCommand);
    expect(input).toHaveValue(typedCommand);

    // Back out to the error banner without saving, then retry again — this
    // second pass through handleRetryBootstrap is the interaction under test.
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await retryBootstrap();

    expect(screen.getByPlaceholderText('e.g. npm test or cargo test')).toHaveValue(typedCommand);
  });

  it('falls back to ext.test_command, then strategy.test_command, when nothing was typed', async () => {
    mockRetryBootstrapBackend();
    mount(baseProject({ status: 'error' }));

    await retryBootstrap();

    // ext.test_command (EXISTING_TEST_COMMAND) wins over strategy.test_command
    // (STRATEGY_TEST_COMMAND) — unchanged pre-fix fallback behavior.
    expect(screen.getByPlaceholderText('e.g. npm test or cargo test')).toHaveValue(EXISTING_TEST_COMMAND);
  });
});

// Regression tests for the same `handleRetryBootstrap` precedence bug, but for
// `defaultBranch`/`branchPrefix`. Unlike `testCommand`/`prTemplate`, these two
// used to be seeded with non-empty placeholders (`'main'` /
// `'demeteo/features/'`) that are always truthy, so `currentDefaultBranch ||
// ext?.default_branch || strategy.default_branch` never fell through to the
// detected/persisted value — the popup showed the hardcoded placeholder for
// any repo whose real default branch wasn't literally `main`.
describe('ProjectHome — handleRetryBootstrap defaultBranch/branchPrefix precedence', () => {
  const EXISTING_DEFAULT_BRANCH = 'develop';
  const STRATEGY_DEFAULT_BRANCH = 'staging';
  const EXISTING_BRANCH_PREFIX = 'feature/';
  const STRATEGY_BRANCH_PREFIX = 'df/';

  function mockRetryBootstrapBackend() {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'fetch_active_features':
          return Promise.resolve([]);
        case 'get_repositories_for_project':
          return Promise.resolve([]);
        case 'workflow_list':
          return Promise.resolve([]);
        case 'remote_list_mirrored_runs':
          return Promise.resolve([]);
        case 'get_proposed_strategy':
          return Promise.resolve({
            worktree_strategy: {
              default_branch: EXISTING_DEFAULT_BRANCH,
              branch_prefix: EXISTING_BRANCH_PREFIX,
              test_command: null,
              pr_template: null,
            },
          });
        case 'bootstrap_project':
          return Promise.resolve({
            default_branch: STRATEGY_DEFAULT_BRANCH,
            branch_prefix: STRATEGY_BRANCH_PREFIX,
            test_command: null,
            pr_template: null,
          });
        default:
          return Promise.resolve(undefined);
      }
    });
  }

  async function retryBootstrap() {
    await userEvent.click(await screen.findByRole('button', { name: /Retry Bootstrap/i }));
    await screen.findByText('STRATEGY DETECTED');
  }

  // The label isn't wired to the input via htmlFor/id — it's a plain sibling
  // — so getByLabelText can't resolve it. Walk from the label text to the
  // input in the same wrapper div instead.
  function getInputByLabel(text: string): HTMLInputElement {
    const label = screen.getByText(text);
    return label.parentElement!.querySelector('input') as HTMLInputElement;
  }

  it('shows the freshly re-detected/persisted branch values, not the old "main"/"demeteo/features/" placeholders, on the first call', async () => {
    mockRetryBootstrapBackend();
    mount(baseProject({ status: 'error' }));

    await retryBootstrap();

    // ext.default_branch / ext.branch_prefix win over strategy.* — same
    // fallback precedence already proven for testCommand above.
    expect(getInputByLabel('Default Branch')).toHaveValue(EXISTING_DEFAULT_BRANCH);
    expect(getInputByLabel('Branch Prefix')).toHaveValue(EXISTING_BRANCH_PREFIX);
  });

  it('preserves a defaultBranch/branchPrefix edit across cancel-then-retry', async () => {
    mockRetryBootstrapBackend();
    mount(baseProject({ status: 'error' }));

    // First pass: open the popup, then simulate a prior user edit.
    await retryBootstrap();
    const typedBranch = 'release/2026';
    const typedPrefix = 'wt/';
    await userEvent.clear(getInputByLabel('Default Branch'));
    await userEvent.type(getInputByLabel('Default Branch'), typedBranch);
    await userEvent.clear(getInputByLabel('Branch Prefix'));
    await userEvent.type(getInputByLabel('Branch Prefix'), typedPrefix);
    expect(getInputByLabel('Default Branch')).toHaveValue(typedBranch);
    expect(getInputByLabel('Branch Prefix')).toHaveValue(typedPrefix);

    // Back out without saving, then retry again — the edited values must
    // survive this second pass through handleRetryBootstrap.
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await retryBootstrap();

    expect(getInputByLabel('Default Branch')).toHaveValue(typedBranch);
    expect(getInputByLabel('Branch Prefix')).toHaveValue(typedPrefix);
  });
});

/**
 * The project header and the pipeline list, wired in Phase 4 of
 * `docs/UI_REDESIGN_PLAN.md`.
 *
 * The provenance claim is audit F10's regression guard, and it is a claim about
 * a *lie* rather than about formatting: the header used to name a provider
 * edition and a default workflow for every project regardless of either, and a
 * project has no default workflow to name at all. Asserting the truthful string
 * would pass on a template that happened to contain it, so the assertions on
 * what must be absent carry the finding.
 */
function ProvenanceSeed({ project, providers, children }: {
  project: Project;
  providers: Provider[];
  children: ReactNode;
}): ReactElement | null {
  const { dispatch } = useProject();
  const [seeded, setSeeded] = useState(false);
  useEffect(() => {
    dispatch({ type: 'LOAD_PROJECTS', projects: [project], reposByProject: {} });
    dispatch({ type: 'SET_CURRENT', id: project.id });
    dispatch({ type: 'SET_PROVIDERS', providers });
    setSeeded(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  if (!seeded) return null;
  return <>{children}</>;
}

function mountWithProviders(project: Project, providers: Provider[]) {
  render(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <TerminalPanelProvider>
            <ProvenanceSeed project={project} providers={providers}>
              <ProjectHome />
            </ProvenanceSeed>
          </TerminalPanelProvider>
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );
}

function provider(overrides: Partial<Provider> = {}): Provider {
  return {
    id: 'provider-1',
    type: 'github',
    name: 'GitHub',
    host: 'git.acme.dev',
    pat: '',
    username: 'octo',
    avatarUrl: '',
    ...overrides,
  };
}

function feature(overrides: Partial<Feature> = {}): Feature {
  return {
    id: 'f-1',
    project_id: 'proj-1',
    title: 'Feature one',
    status: 'completed',
    total_cost: 0,
    tokens: 0,
    duration: '1m',
    created_at: 1,
    ...overrides,
  };
}

describe('the project header’s provenance line', () => {
  beforeEach(() => {
    mockBackend(['/repo/a']);
  });

  it('names the provider the project is actually connected to', async () => {
    mountWithProviders(baseProject(), [provider()]);

    const line = await screen.findByTestId('project-provenance');
    expect(line).toHaveTextContent('Connected via GitHub (git.acme.dev)');
  });

  it('never claims an edition or a default workflow', async () => {
    mountWithProviders(baseProject(), [provider()]);

    const line = await screen.findByTestId('project-provenance');
    // Both halves of F10. A self-hosted host is *probably* Enterprise, and
    // "probably" is what the finding was made of; a default workflow is not a
    // property a project has at all.
    expect(line.textContent).not.toMatch(/enterprise/i);
    expect(line.textContent).not.toMatch(/workflow/i);
  });

  it('names the project’s chosen default workflow', async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'workflow_list':
          return Promise.resolve([{ id: 'wf-2', name: 'Docs Update', is_starter: true }]);
        case 'get_proposed_strategy':
          return Promise.resolve({ project_id: 'proj-1', default_workflow_id: 'wf-2' });
        case 'get_repositories_for_project':
          return Promise.resolve([]);
        default:
          return Promise.resolve([]);
      }
    });
    mountWithProviders(baseProject(), []);

    await waitFor(() =>
      expect(screen.getByTestId('project-provenance')).toHaveTextContent(
        'Default workflow: Docs Update',
      ),
    );
  });

  it('omits the workflow clause when the chosen one has been deleted', async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'workflow_list':
          return Promise.resolve([{ id: 'wf-2', name: 'Docs Update', is_starter: true }]);
        case 'get_proposed_strategy':
          // The column carries no foreign key on purpose, so a deleted
          // workflow leaves this id behind — reachable in normal use.
          return Promise.resolve({ project_id: 'proj-1', default_workflow_id: 'wf-gone' });
        case 'get_repositories_for_project':
          return Promise.resolve([]);
        default:
          return Promise.resolve([]);
      }
    });
    mountWithProviders(baseProject(), []);

    await waitFor(() => expect(screen.getByTestId('project-provenance')).toBeInTheDocument());
    expect(screen.getByTestId('project-provenance').textContent).not.toMatch(/workflow/i);
  });

  it('says where the run executes rather than inventing a machine', async () => {
    mountWithProviders(baseProject({ compute_type: 'remote' }), [provider()]);

    const line = await screen.findByTestId('project-provenance');
    expect(line).toHaveTextContent('Runs remotely');
  });
});

describe('the pipeline list’s filter', () => {
  const FEATURES: Feature[] = [
    feature({ id: 'f-done', title: 'Landed already', status: 'completed' }),
    feature({ id: 'f-gate', title: 'Waiting on a human', status: 'gated' }),
    feature({ id: 'f-run', title: 'Still moving', status: 'running' }),
  ];

  beforeEach(() => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'fetch_active_features':
          return Promise.resolve(FEATURES);
        case 'get_repositories_for_project':
          return Promise.resolve([]);
        default:
          return Promise.resolve([]);
      }
    });
  });

  it('shows every pipeline until a segment is chosen', async () => {
    mountWithProviders(baseProject(), []);

    expect(await screen.findByText('Landed already')).toBeInTheDocument();
    expect(screen.getByText('Waiting on a human')).toBeInTheDocument();
    expect(screen.getByText('Still moving')).toBeInTheDocument();
  });

  it('narrows the rendered rows to the chosen segment', async () => {
    mountWithProviders(baseProject(), []);
    await screen.findByText('Landed already');

    await userEvent.click(screen.getByRole('radio', { name: /needs you/i }));

    // The filter has to reach the rendered list, not merely the control: the
    // policy is already covered in `pipelineFilter.test.ts`, so what is worth
    // asserting here is that `ProjectHome` renders what it returns.
    expect(screen.getByText('Waiting on a human')).toBeInTheDocument();
    expect(screen.queryByText('Landed already')).not.toBeInTheDocument();
    expect(screen.queryByText('Still moving')).not.toBeInTheDocument();

    // Waited for, not merely allowed: the write a segment click schedules is
    // debounced and held on a module-scoped preference, so it outlives this
    // component. Left in flight it fires 400 ms later, inside whatever test is
    // running by then, and `clearAllMocks` has made it look like that test's.
    await waitFor(() =>
      expect(commandsOf('set_app_session')).toEqual([
        { key: 'ui.pipeline_segment', value: 'needs-you' },
      ]),
    );
  });
});

/**
 * What the project view remembers between launches (`docs/UI_REDESIGN_PLAN.md`
 * §6 Phase 6). Every stored row seeded here differs from the in-memory default,
 * so a build that ignored the store — or wrote over it on first paint — fails
 * these rather than passing on a value that happens to match.
 *
 * Fake timers, and therefore `fireEvent` and no `findBy*`: the write debounce is
 * a `setTimeout`, and Testing Library's async helpers do not detect vitest's
 * clock, so a `waitFor` here would poll on an interval that never fires.
 */
describe('the project view’s persisted list preferences', () => {
  const FEATURES: Feature[] = [
    feature({ id: 'f-done', title: 'Landed already', status: 'completed', created_at: 3 }),
    feature({ id: 'f-gate', title: 'Waiting on a human', status: 'gated', created_at: 2 }),
    feature({ id: 'f-run', title: 'Still moving', status: 'running', created_at: 1 }),
  ];

  function mockStore(rows: Record<string, string>) {
    vi.mocked(invoke).mockImplementation((cmd: string, args?: InvokeArgs) => {
      switch (cmd) {
        case 'fetch_active_features':
          return Promise.resolve(FEATURES);
        case 'get_app_session': {
          const { key } = (args ?? {}) as { key?: string };
          return Promise.resolve(rows[key ?? ''] ?? null);
        }
        default:
          return Promise.resolve([]);
      }
    });
  }

  function writes(): Array<{ key: string; value: string }> {
    return commandsOf('set_app_session').map((args) => ({
      key: String(args?.key),
      value: String(args?.value),
    }));
  }

  /** Resolve the mounted reads and run out any debounce they armed. */
  async function settle() {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(UI_PREF_WRITE_DEBOUNCE_MS);
    });
  }

  function queryBox(): HTMLElement {
    return screen.getByLabelText('Filter pipelines by text');
  }

  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('opens on the segment and the sort the last session left behind', async () => {
    mockStore({ 'ui.pipeline_segment': 'needs-you', 'ui.pipeline_sort': 'oldest' });
    mount(baseProject({ compute_type: 'local' }));
    await settle();

    expect(screen.getByText('Waiting on a human')).toBeInTheDocument();
    expect(screen.queryByText('Landed already')).not.toBeInTheDocument();
    expect(screen.getByLabelText('Sort pipelines')).toHaveValue('oldest');
  });

  it('opens compact when compact is what was stored, and stores the next choice', async () => {
    mockStore({ 'ui.density': 'compact' });
    mount(baseProject({ compute_type: 'local' }));
    await settle();

    expect(screen.getByRole('radio', { name: 'Compact' })).toHaveAttribute('aria-checked', 'true');

    fireEvent.click(screen.getByRole('radio', { name: 'Comfortable' }));
    await settle();

    expect(writes()).toEqual([{ key: 'ui.density', value: 'comfortable' }]);
  });

  it('stores nothing when the view merely opens', async () => {
    mockStore({ 'ui.density': 'compact', 'ui.pipeline_segment': 'needs-you' });
    mount(baseProject({ compute_type: 'local' }));
    await settle();

    expect(commandsOf('get_app_session').length).toBeGreaterThan(0);
    expect(writes()).toEqual([]);
  });

  it('filters on the typed query without ever storing it', async () => {
    mockStore({});
    mount(baseProject({ compute_type: 'local' }));
    await settle();

    fireEvent.change(queryBox(), { target: { value: 'waiting' } });
    await settle();

    expect(screen.getByText('Waiting on a human')).toBeInTheDocument();
    expect(screen.queryByText('Still moving')).not.toBeInTheDocument();
    expect(writes()).toEqual([]);
  });

  it('stores the cleared segment when the reset link puts the list back', async () => {
    mockStore({ 'ui.pipeline_segment': 'needs-you' });
    mount(baseProject({ compute_type: 'local' }));
    await settle();

    fireEvent.change(queryBox(), { target: { value: 'matches nothing at all' } });
    await settle();
    expect(writes()).toEqual([]);

    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }));
    await settle();

    expect(writes()).toEqual([{ key: 'ui.pipeline_segment', value: 'all' }]);
    expect(screen.getByText('Landed already')).toBeInTheDocument();
  });
});
