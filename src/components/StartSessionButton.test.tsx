import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { type ReactElement } from 'react';
import { describe, expect, it, beforeEach, vi } from 'vitest';
import { invoke, type InvokeArgs } from '@tauri-apps/api/core';

import { NavigationProvider, useNavigation } from '../context/NavigationContext';
import { TerminalPanelProvider } from '../context/TerminalPanelProvider';
import { StartSessionButton } from './StartSessionButton';

let nextSessionId = 0;

beforeEach(() => {
  nextSessionId = 0;
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation((cmd: string, _args?: InvokeArgs) => {
    if (cmd === 'list_terminal_sessions') return Promise.resolve([]);
    if (cmd === 'resolve_repo_dir') return Promise.resolve('/tmp/repo');
    if (cmd === 'start_terminal_session') {
      return Promise.resolve({ session_id: `sess_${++nextSessionId}`, launch_command: null });
    }
    return Promise.resolve(undefined);
  });
});

function commandArgs(name: string): Array<InvokeArgs | undefined> {
  return vi
    .mocked(invoke)
    .mock.calls.filter(([c]) => c === name)
    .map(([, args]) => args as InvokeArgs | undefined);
}

function NavProbe(): ReactElement {
  const { view } = useNavigation();
  return <span data-testid="nav-view">{view.kind}</span>;
}

function mount() {
  render(
    <NavigationProvider>
      <TerminalPanelProvider>
        <NavProbe />
        <StartSessionButton
          projectId="proj-1"
          repoPath="/repo/one"
          machineId="local"
          machineLabel="local"
        />
      </TerminalPanelProvider>
    </NavigationProvider>,
  );
}

describe('StartSessionButton', () => {
  it('primary click opens a plain shell (no agentKind) and navigates to terminals', async () => {
    mount();
    await act(async () => {
      await userEvent.click(screen.getByTestId('start-session-primary'));
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });

    const started = commandArgs('start_terminal_session');
    expect(started).toHaveLength(1);
    expect(started[0]).toMatchObject({
      machineId: 'local',
      workDir: '/tmp/repo',
      agentKind: null,
      launchCommand: null,
    });

    const resolved = commandArgs('resolve_repo_dir');
    expect(resolved[0]).toMatchObject({ projectId: 'proj-1', repoPath: '/repo/one' });

    expect(screen.getByTestId('nav-view').textContent).toBe('terminals');
  });

  it('caret dropdown lists the AGENTS registry and launches with kind/binary as agentKind/launchCommand', async () => {
    mount();
    await userEvent.click(screen.getByTestId('start-session-caret'));
    expect(screen.getByTestId('start-session-dropdown')).toBeInTheDocument();
    expect(screen.getByTestId('start-session-agent-claude-code')).toHaveTextContent('Claude');
    expect(screen.getByTestId('start-session-agent-opencode')).toHaveTextContent('OpenCode');

    await act(async () => {
      await userEvent.click(screen.getByTestId('start-session-agent-claude-code'));
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });

    const started = commandArgs('start_terminal_session');
    expect(started).toHaveLength(1);
    expect(started[0]).toMatchObject({
      machineId: 'local',
      workDir: '/tmp/repo',
      agentKind: 'claude-code',
      launchCommand: 'claude',
    });
    expect(screen.getByTestId('nav-view').textContent).toBe('terminals');
  });

  it('disables both buttons while an open() call is in flight', async () => {
    let resolveStart: (() => void) | null = null;
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === 'list_terminal_sessions') return Promise.resolve([]);
      if (cmd === 'resolve_repo_dir') return Promise.resolve('/tmp/repo');
      if (cmd === 'start_terminal_session') {
        return new Promise((resolve) => {
          resolveStart = () => resolve({ session_id: 'sess_pending', launch_command: null });
        });
      }
      return Promise.resolve(undefined);
    });
    mount();

    await act(async () => {
      await userEvent.click(screen.getByTestId('start-session-primary'));
      await Promise.resolve();
    });

    expect(screen.getByTestId('start-session-primary')).toBeDisabled();
    expect(screen.getByTestId('start-session-caret')).toBeDisabled();

    await act(async () => {
      resolveStart?.();
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });

    expect(screen.getByTestId('start-session-primary')).not.toBeDisabled();
  });

  it('renders a visible inline error when open() rejects, and clears it on the next successful click', async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === 'list_terminal_sessions') return Promise.resolve([]);
      if (cmd === 'resolve_repo_dir') return Promise.resolve('/tmp/repo');
      if (cmd === 'start_terminal_session') {
        return Promise.reject({ kind: 'io_error', message: 'boom: failed to spawn shell' });
      }
      return Promise.resolve(undefined);
    });
    mount();

    await act(async () => {
      await userEvent.click(screen.getByTestId('start-session-primary'));
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });

    expect(screen.getByTestId('start-session-error')).toHaveTextContent(
      'boom: failed to spawn shell',
    );
    // The failed attempt must not have navigated away.
    expect(screen.getByTestId('nav-view').textContent).not.toBe('terminals');

    // Next click succeeds — the stale error must clear.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === 'list_terminal_sessions') return Promise.resolve([]);
      if (cmd === 'resolve_repo_dir') return Promise.resolve('/tmp/repo');
      if (cmd === 'start_terminal_session') {
        return Promise.resolve({ session_id: 'sess_recover', launch_command: null });
      }
      return Promise.resolve(undefined);
    });

    await act(async () => {
      await userEvent.click(screen.getByTestId('start-session-primary'));
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });

    expect(screen.queryByTestId('start-session-error')).not.toBeInTheDocument();
    expect(screen.getByTestId('nav-view').textContent).toBe('terminals');
  });
});
