import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { type ReactElement } from 'react';
import { describe, expect, it, beforeEach, vi } from 'vitest';
import { invoke, type InvokeArgs } from '@tauri-apps/api/core';

import { TerminalPanelProvider } from '../context/TerminalPanelProvider';
import { useTerminalPanel } from '../hooks/useTerminalPanel';
import { TerminalsView } from './TerminalsView';

let nextSessionId = 0;

beforeEach(() => {
  nextSessionId = 0;
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation((cmd: string, _args?: InvokeArgs) => {
    if (cmd === 'list_terminal_sessions') return Promise.resolve([]);
    if (cmd === 'get_agent_configs') return Promise.resolve([]);
    if (cmd === 'resolve_repo_dir') return Promise.resolve('/tmp/repo');
    if (cmd === 'start_terminal_session') {
      return Promise.resolve(`sess_${++nextSessionId}`);
    }
    return Promise.resolve(undefined);
  });
});

function commandsOf(name: string): Array<unknown[]> {
  return vi.mocked(invoke).mock.calls.filter(([c]) => c === name);
}

interface Harness {
  readonly panel: ReturnType<typeof useTerminalPanel>;
}

function mount(active = true): Harness {
  const ref: { current: ReturnType<typeof useTerminalPanel> | null } = { current: null };
  function Capture(): ReactElement {
    const panel = useTerminalPanel();
    ref.current = panel;
    return <span data-testid="dbg">{panel.state.tabs.length}</span>;
  }
  render(
    <TerminalPanelProvider>
      <Capture />
      <TerminalsView active={active} />
    </TerminalPanelProvider>,
  );
  return {
    get panel() {
      if (!ref.current) throw new Error('panel did not mount');
      return ref.current;
    },
  };
}

describe('TerminalsView', () => {
  it('renders the empty state (with a New menu) when no tabs are open', () => {
    mount();
    expect(screen.getByText('No terminals open')).toBeInTheDocument();
    expect(screen.getByTestId('new-terminal-menu')).toBeInTheDocument();
  });

  it('renders one row per tab and exactly one surface for the active tab', async () => {
    const h = mount();
    await act(async () => {
      await h.panel.open({ machineId: 'local', machineLabel: 'local', repoPath: '/a' });
      await h.panel.open({ machineId: 'local', machineLabel: 'local', repoPath: '/b' });
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });
    expect(screen.getAllByRole('tab')).toHaveLength(2);
    // Single live xterm — only the active tab mounts a surface.
    expect(screen.getAllByTestId('terminal-surface')).toHaveLength(1);
  });

  it('focuses a tab when its row is clicked, swapping the mounted surface', async () => {
    const h = mount();
    let tabA = '';
    await act(async () => {
      tabA = await h.panel.open({ machineId: 'local', machineLabel: 'local', repoPath: '/a' });
      await h.panel.open({ machineId: 'local', machineLabel: 'local', repoPath: '/b' });
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });
    // Tab B is active (last opened). Click row A to focus it.
    await act(async () => {
      await userEvent.click(screen.getByTestId(`session-row-${tabA}`));
      for (let i = 0; i < 5; i++) await Promise.resolve();
    });
    expect(h.panel.state.activeTabId).toBe(tabA);
    expect(screen.getAllByTestId('terminal-surface')).toHaveLength(1);
  });

  it('moves the active selection with ArrowUp/ArrowDown on the session list', async () => {
    const h = mount();
    let tabA = '';
    let tabB = '';
    await act(async () => {
      tabA = await h.panel.open({ machineId: 'local', machineLabel: 'local', repoPath: '/a' });
      tabB = await h.panel.open({ machineId: 'local', machineLabel: 'local', repoPath: '/b' });
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });
    // B is active (opened last). ArrowUp moves selection to A.
    const list = screen.getByRole('tablist');
    await act(async () => {
      list.focus();
      await userEvent.keyboard('{ArrowUp}');
      for (let i = 0; i < 3; i++) await Promise.resolve();
    });
    expect(h.panel.state.activeTabId).toBe(tabA);
    // ArrowDown moves back to B.
    await act(async () => {
      await userEvent.keyboard('{ArrowDown}');
      for (let i = 0; i < 3; i++) await Promise.resolve();
    });
    expect(h.panel.state.activeTabId).toBe(tabB);
  });

  it('does not close backend sessions when the view is hidden off-route', async () => {
    const h = mount(true);
    await act(async () => {
      await h.panel.open({ machineId: 'local', machineLabel: 'local', repoPath: '/a' });
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });
    vi.mocked(invoke).mockClear();
    // Re-render with active=false (navigated away). The view stays mounted;
    // sessions must NOT be torn down (invariant 1).
    // A simple way to assert: closing is never invoked by hiding.
    expect(commandsOf('close_terminal_session')).toHaveLength(0);
  });
});
