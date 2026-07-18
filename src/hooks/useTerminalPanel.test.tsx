import { act, render } from '@testing-library/react';
import {
  useEffect,
  type ReactElement,
} from 'react';
import { describe, expect, it, beforeEach, vi } from 'vitest';
import { invoke, type InvokeArgs } from '@tauri-apps/api/core';

import { TerminalPanelProvider } from '../context/TerminalPanelProvider';
import { useTerminalPanel } from '../hooks/useTerminalPanel';
import type { SessionInfo } from '../types';

let nextSessionId = 0;
const nextId = (): string => `sess_${++nextSessionId}`;

interface StartedSession {
  sessionId: string;
  args: InvokeArgs;
}
const startedSessions: StartedSession[] = [];

beforeEach(() => {
  nextSessionId = 0;
  startedSessions.length = 0;
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation((cmd: string, args?: InvokeArgs) => {
    if (cmd === 'list_terminal_sessions') return Promise.resolve([]);
    if (cmd === 'resolve_repo_dir') return Promise.resolve('/tmp/repo');
    if (cmd === 'start_terminal_session') {
      const sessionId = nextId();
      startedSessions.push({ sessionId, args: args ?? {} });
      return Promise.resolve(sessionId);
    }
    return Promise.resolve(undefined);
  });
});

function commandsOf(name: string): Array<unknown[]> {
  return vi.mocked(invoke).mock.calls.filter(([c]) => c === name);
}

interface PanelHarness {
  readonly panel: ReturnType<typeof useTerminalPanel>;
}

function mountHarness(): PanelHarness {
  const ref: { current: ReturnType<typeof useTerminalPanel> | null } = { current: null };

  function Capture(): ReactElement {
    const panel = useTerminalPanel();
    ref.current = panel;
    // Render the tabs length so React cannot bail out of the re-render
    // when the context value changes.
    return <span data-testid="capture-debug">{panel.state.tabs.length}</span>;
  }

  render(
    <TerminalPanelProvider>
      <Capture />
    </TerminalPanelProvider>,
  );

  return {
    get panel(): ReturnType<typeof useTerminalPanel> {
      if (!ref.current) throw new Error('panel hook did not mount');
      return ref.current;
    },
  };
}

describe('useTerminalPanel — open()', () => {
  it('calls start_terminal_session exactly once on first open', async () => {
    const h = mountHarness();

    await act(async () => {
      await h.panel.open({
        machineId: 'local',
        machineLabel: 'local',
        projectId: 'p1',
        repoPath: 'repo',
      });
    });

    const starts = commandsOf('start_terminal_session');
    expect(starts).toHaveLength(1);
    expect(starts[0][1]).toMatchObject({
      machineId: 'local',
      workBranch: null,
    });
    expect(commandsOf('resolve_repo_dir')).toHaveLength(1);
  });

  it('skips resolveRepoDir when no projectId is supplied (raw PTY open)', async () => {
    const h = mountHarness();

    await act(async () => {
      await h.panel.open({ machineId: 'local', machineLabel: 'local' });
    });

    expect(commandsOf('start_terminal_session')).toHaveLength(1);
    expect(commandsOf('resolve_repo_dir')).toHaveLength(0);
  });

  it('focuses the new tab on OPEN_TAB', async () => {
    const h = mountHarness();

    await act(async () => {
      await h.panel.open({ machineId: 'local', machineLabel: 'local' });
    });

    expect(h.panel.state.activeTabId).toBe(h.panel.state.tabs[0]?.tabId);
  });

  it('focuses the second tab when a second terminal is opened', async () => {
    const h = mountHarness();

    await act(async () => {
      await h.panel.open({
        machineId: 'local',
        machineLabel: 'local',
        projectId: 'p1',
        repoPath: '/srv/repo-a',
      });
    });
    const firstTabId = h.panel.state.tabs[0].tabId;

    await act(async () => {
      await h.panel.open({
        machineId: 'local',
        machineLabel: 'local',
        projectId: 'p1',
        repoPath: '/srv/repo-b',
      });
    });

    expect(h.panel.state.activeTabId).not.toBe(firstTabId);
    expect(h.panel.state.activeTabId).toBe(h.panel.state.tabs[1]?.tabId);
    expect(h.panel.state.tabs).toHaveLength(2);
  });

  it('reuses an existing tab for the same (machineId, repoPath, workBranch) and does NOT issue a second start_terminal_session', async () => {
    const h = mountHarness();

    let firstTabId = '';
    await act(async () => {
      firstTabId = await h.panel.open({
        machineId: 'local',
        machineLabel: 'local',
        projectId: 'p1',
        repoPath: '/srv/repo',
      });
    });

    expect(commandsOf('start_terminal_session')).toHaveLength(1);

    // Second open() with identical params — exercises the
    // ProjectHome TerminalTabOpener re-mount case where the user
    // navigates out and back without closing the terminal.
    let secondTabId = '';
    await act(async () => {
      secondTabId = await h.panel.open({
        machineId: 'local',
        machineLabel: 'local',
        projectId: 'p1',
        repoPath: '/srv/repo',
      });
    });

    expect(secondTabId).toBe(firstTabId);
    expect(commandsOf('start_terminal_session')).toHaveLength(1);
    expect(h.panel.state.tabs).toHaveLength(1);
  });

  it('reuses an existing tab even when workBranch matches and keeps the focused tab focused', async () => {
    const h = mountHarness();

    let firstTabId = '';
    await act(async () => {
      firstTabId = await h.panel.open({
        machineId: 'remote-1',
        machineLabel: 'host-1',
        projectId: 'p1',
        workDir: '/srv/wt-a',
        workBranch: 'feat/login',
      });
    });

    expect(commandsOf('start_terminal_session')).toHaveLength(1);

    // Open a second tab for a DIFFERENT workBranch on the same machine.
    await act(async () => {
      await h.panel.open({
        machineId: 'remote-1',
        machineLabel: 'host-1',
        projectId: 'p1',
        workDir: '/srv/wt-b',
        workBranch: 'feat/checkout',
      });
    });
    expect(commandsOf('start_terminal_session')).toHaveLength(2);
    expect(h.panel.state.tabs).toHaveLength(2);

    // A third open() with the FIRST tab's tuple must reuse the first tab
    // and re-focus it.
    await act(async () => {
      await h.panel.open({
        machineId: 'remote-1',
        machineLabel: 'host-1',
        projectId: 'p1',
        workDir: '/srv/wt-a',
        workBranch: 'feat/login',
      });
    });

    expect(commandsOf('start_terminal_session')).toHaveLength(2);
    expect(h.panel.state.tabs).toHaveLength(2);
    expect(h.panel.state.activeTabId).toBe(firstTabId);
  });

  it('coalesces two concurrent opens for the same logical tab into one start (F3)', async () => {
    const h = mountHarness();

    // Fire two opens for the identical tuple in the SAME tick, before
    // React re-renders `stateRef`. Pre-fix this raced two backend
    // sessions and orphaned one; now the second coalesces onto the
    // first's in-flight promise.
    let idA = '';
    let idB = '';
    await act(async () => {
      const pA = h.panel.open({
        machineId: 'local',
        machineLabel: 'local',
        projectId: 'p1',
        repoPath: '/srv/repo',
      });
      const pB = h.panel.open({
        machineId: 'local',
        machineLabel: 'local',
        projectId: 'p1',
        repoPath: '/srv/repo',
      });
      [idA, idB] = await Promise.all([pA, pB]);
    });

    // Exactly one backend session, one tab, and both callers got the
    // same tabId.
    expect(commandsOf('start_terminal_session')).toHaveLength(1);
    expect(h.panel.state.tabs).toHaveLength(1);
    expect(idB).toBe(idA);
  });

  it('forceNew bypasses dedup and stacks a second session on the same tuple (F3/§7)', async () => {
    const h = mountHarness();

    await act(async () => {
      await h.panel.open({
        machineId: 'local',
        machineLabel: 'local',
        projectId: 'p1',
        repoPath: '/srv/repo',
      });
    });
    expect(commandsOf('start_terminal_session')).toHaveLength(1);

    // A forceNew open for the same tuple must start a second session
    // and add a second tab rather than reuse the first.
    await act(async () => {
      await h.panel.open({
        machineId: 'local',
        machineLabel: 'local',
        projectId: 'p1',
        repoPath: '/srv/repo',
        forceNew: true,
      });
    });
    expect(commandsOf('start_terminal_session')).toHaveLength(2);
    expect(h.panel.state.tabs).toHaveLength(2);
  });
});

describe('useTerminalPanel — close()', () => {
  it('calls close_terminal_session exactly once with the session id the backend returned', async () => {
    const h = mountHarness();

    let tabId = '';
    await act(async () => {
      tabId = await h.panel.open({ machineId: 'local', machineLabel: 'local' });
    });

    expect(tabId).toBeTruthy();
    expect(startedSessions).toHaveLength(1);
    const startedSessionId = startedSessions[0].sessionId;

    vi.mocked(invoke).mockClear();
    startedSessions.length = 0;

    await act(async () => {
      await h.panel.close(tabId);
    });

    const closes = commandsOf('close_terminal_session');
    expect(closes).toHaveLength(1);
    expect((closes[0][1] as { sessionId: string }).sessionId).toBe(startedSessionId);
  });

  it('tears down an orphaned backend session when close races start', async () => {
    const resolveStartRef: { current: ((sid: string) => void) | null } = { current: null };
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === 'list_terminal_sessions') return Promise.resolve([]);
      if (cmd === 'resolve_repo_dir') return Promise.resolve('/tmp/repo');
      if (cmd === 'start_terminal_session') {
        return new Promise<string>((resolve) => {
          resolveStartRef.current = (sid) => resolve(sid);
        });
      }
      if (cmd === 'close_terminal_session') {
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });

    const h = mountHarness();
    const openPromise = h.panel.open({ machineId: 'local', machineLabel: 'local' });

    // Drain microtasks until the OPEN_TAB dispatch commits so we can
    // read the tabId from state.
    await act(async () => {
      for (let i = 0; i < 5; i++) {
        await Promise.resolve();
      }
    });

    expect(h.panel.state.tabs).toHaveLength(1);
    const tabId = h.panel.state.tabs[0].tabId;

    await act(async () => {
      await h.panel.close(tabId);
    });

    resolveStartRef.current?.('sess_race');
    await openPromise.catch(() => {});

    await act(async () => {
      for (let i = 0; i < 5; i++) {
        await Promise.resolve();
      }
    });

    const closes = commandsOf('close_terminal_session');
    expect(closes.length).toBeGreaterThanOrEqual(1);
    const orphanedCleanup = closes.find(
      ([, args]) => (args as { sessionId: string }).sessionId === 'sess_race',
    );
    expect(orphanedCleanup).toBeDefined();
    expect(h.panel.state.tabs).toHaveLength(0);
  });
});

describe('useTerminalPanel — resolveRepoDir failure', () => {
  it('fails closed: tab is marked error and the backend is not started', async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === 'list_terminal_sessions') return Promise.resolve([]);
      if (cmd === 'resolve_repo_dir') return Promise.reject('repo not found');
      return Promise.resolve(undefined);
    });

    const h = mountHarness();

    await act(async () => {
      await expect(
        h.panel.open({
          machineId: 'local',
          machineLabel: 'local',
          projectId: 'p1',
          repoPath: 'missing',
        }),
      ).rejects.toBeDefined();
    });

    expect(commandsOf('start_terminal_session')).toHaveLength(0);
    expect(h.panel.state.tabs[0]?.phase).toBe('error');
  });
});

describe('useTerminalPanel — reconnect()', () => {
  it('calls reconnect_terminal_session once and coalesces concurrent double-clicks', async () => {
    const h = mountHarness();

    let tabId = '';
    await act(async () => {
      tabId = await h.panel.open({ machineId: 'local', machineLabel: 'local' });
    });

    // Two rapid reconnect() calls for the same tab — the in-flight guard
    // must collapse them to a single backend call (spec §4.4).
    await act(async () => {
      await Promise.all([h.panel.reconnect(tabId), h.panel.reconnect(tabId)]);
    });

    expect(commandsOf('reconnect_terminal_session')).toHaveLength(1);
    expect(h.panel.state.tabs[0]?.phase).toBe('running');
  });

  it('no-ops for a tab that has no backend session yet', async () => {
    const h = mountHarness();
    await act(async () => {
      await h.panel.reconnect('never-opened');
    });
    expect(commandsOf('reconnect_terminal_session')).toHaveLength(0);
  });
});

describe('useTerminalPanel — startup reconciliation', () => {
  it('rehydrates tabs from list_terminal_sessions on mount, sorted by created_at', async () => {
    const restored: SessionInfo[] = [
      { session_id: 'sess_b', machine_id: 'local', created_at: 200, title: 'build' },
      { session_id: 'sess_a', machine_id: 'local', created_at: 100, title: null },
    ];
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === 'list_terminal_sessions') return Promise.resolve(restored);
      return Promise.resolve(undefined);
    });

    const h = mountHarness();

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(commandsOf('list_terminal_sessions')).toHaveLength(1);
    expect(h.panel.state.tabs).toHaveLength(2);
    expect(h.panel.state.tabs[0]?.sessionId).toBe('sess_a');
    expect(h.panel.state.tabs[1]?.sessionId).toBe('sess_b');
    // Restored sessions are alive PTYs (they only appear if the backend
    // still holds them), so they reconcile as `running`, not `closed`
    // (finding F6).
    expect(h.panel.state.tabs[0]?.phase).toBe('running');
    expect(h.panel.state.tabs[1]?.phase).toBe('running');
    expect(h.panel.state.tabs[1]?.title).toBe('build');
    expect(h.panel.state.tabs[0]?.title).toBe('local');
  });

  it('merges restored tabs instead of clobbering user-initiated tabs in the race window', async () => {
    const restored: SessionInfo[] = [
      { session_id: 'sess_legacy', machine_id: 'local', created_at: 100, title: 'restored' },
    ];
    let resolveList: ((sessions: SessionInfo[]) => void) | null = null;
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === 'list_terminal_sessions') {
        return new Promise<SessionInfo[]>((resolve) => {
          resolveList = resolve;
        });
      }
      if (cmd === 'start_terminal_session') return Promise.resolve('sess_user');
      return Promise.resolve(undefined);
    });

    const h = mountHarness();

    // User clicks "open terminal" before the startup reconcile response
    // resolves. This is the race window the previous implementation
    // clobbered (replaced state.tabs with the restored set).
    let userTabId = '';
    await act(async () => {
      userTabId = await h.panel.open({ machineId: 'local', machineLabel: 'local' });
      await Promise.resolve();
    });

    expect(h.panel.state.tabs).toHaveLength(1);
    expect(h.panel.state.tabs[0]?.tabId).toBe(userTabId);

    // Now the reconcile response arrives.
    await act(async () => {
      resolveList!(restored);
      await Promise.resolve();
      await Promise.resolve();
    });

    // The user-initiated tab is preserved AND the restored tab is appended.
    expect(h.panel.state.tabs).toHaveLength(2);
    expect(h.panel.state.tabs.map((t) => t.sessionId)).toEqual([
      'sess_user',
      'sess_legacy',
    ]);
    expect(h.panel.state.tabs[0]?.tabId).toBe(userTabId);
  });
});

describe('useTerminalPanel — view unmount safety', () => {
  function OpenOnMount({
    machineId,
    projectId,
    repoPath,
  }: {
    machineId: string;
    projectId?: string;
    repoPath?: string;
  }): ReactElement {
    const panel = useTerminalPanel();
    const open = panel.open;
    useEffect(() => {
      void open({ machineId, machineLabel: machineId, projectId, repoPath });
    }, [open, machineId, projectId, repoPath]);
    return <></>;
  }

  it('does NOT call close_terminal_session when the consumer unmounts', async () => {
    let resolver: (() => void) | null = null;
    const waitFor = new Promise<void>((resolve) => {
      resolver = resolve;
    });

    function Harness(): ReactElement {
      useEffect(() => {
        resolver?.();
      }, []);
      return (
        <OpenOnMount machineId="local" projectId="p1" repoPath="repo" />
      );
    }

    const view = render(
      <TerminalPanelProvider>
        <Harness />
      </TerminalPanelProvider>,
    );

    await act(async () => {
      await waitFor;
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(commandsOf('start_terminal_session')).toHaveLength(1);

    vi.mocked(invoke).mockClear();

    await act(async () => {
      view.unmount();
      await Promise.resolve();
    });

    expect(commandsOf('close_terminal_session')).toHaveLength(0);
    expect(commandsOf('detach_terminal_session')).toHaveLength(0);
  });
});

describe('useTerminalPanel — focus() rebinds detached tabs', () => {
  it('re-focusing a detached tab triggers attach_terminal_session exactly once', async () => {
    const { TerminalsView } = await import('../components/TerminalsView');
    const { screen } = await import('@testing-library/react');

    const ref: { current: ReturnType<typeof useTerminalPanel> | null } = { current: null };

    function Host(): ReactElement {
      const panel = useTerminalPanel();
      ref.current = panel;
      // The Terminals view mounts exactly one surface (the active tab)
      // and remounts it on focus — the same single-surface contract the
      // retired panel host had.
      return <TerminalsView active />;
    }

    render(
      <TerminalPanelProvider>
        <Host />
      </TerminalPanelProvider>,
    );

    let tabA = '';
    await act(async () => {
      tabA = await ref.current!.open({
        machineId: 'local',
        machineLabel: 'local',
        projectId: 'p1',
        repoPath: '/srv/repo-a',
      });
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });

    await act(async () => {
      await ref.current!.open({
        machineId: 'local',
        machineLabel: 'local',
        projectId: 'p1',
        repoPath: '/srv/repo-b',
      });
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });

    expect(screen.getAllByTestId('terminal-surface')).toHaveLength(1);
    expect(commandsOf('attach_terminal_session').length).toBe(2);
    expect(commandsOf('detach_terminal_session').length).toBe(1);

    vi.mocked(invoke).mockClear();

    await act(async () => {
      ref.current!.focus(tabA);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(screen.getAllByTestId('terminal-surface')).toHaveLength(1);
    expect(commandsOf('attach_terminal_session').length).toBe(1);
    expect(commandsOf('detach_terminal_session').length).toBe(1);
  });
});

describe('useTerminalPanel — default title', () => {
  it('uses the repoPath basename when one is supplied', async () => {
    const h = mountHarness();
    await act(async () => {
      await h.panel.open({
        machineId: 'local',
        machineLabel: 'local',
        repoPath: '/home/me/projects/cool-app',
      });
    });
    expect(h.panel.state.tabs[0]?.title).toBe('cool-app');
  });

  it('falls back to a numbered "terminal N" when no repoPath or label is meaningful', async () => {
    const h = mountHarness();
    await act(async () => {
      await h.panel.open({ machineId: 'local', machineLabel: '' });
    });
    expect(h.panel.state.tabs[0]?.title).toBe('terminal 1');

    await act(async () => {
      await h.panel.open({ machineId: 'remote-1', machineLabel: '' });
    });
    expect(h.panel.state.tabs[1]?.title).toBe('terminal 2');
  });
});

describe('useTerminalPanel — workDir bypass', () => {
  it('uses the explicit workDir verbatim and skips resolve_repo_dir (feature worktrees)', async () => {
    const h = mountHarness();

    await act(async () => {
      await h.panel.open({
        machineId: 'local',
        machineLabel: 'local',
        projectId: 'p1',
        workDir: '/srv/workspaces/demeteo-wt-abc123',
      });
    });

    const starts = commandsOf('start_terminal_session');
    expect(starts).toHaveLength(1);
    expect(starts[0][1]).toMatchObject({
      machineId: 'local',
      workDir: '/srv/workspaces/demeteo-wt-abc123',
    });
    expect(commandsOf('resolve_repo_dir')).toHaveLength(0);
  });
});

describe('useTerminalPanel — getSessionId', () => {
  it('resolves the backend session id from a frontend tabId once start resolves', async () => {
    const h = mountHarness();
    let tabId = '';
    await act(async () => {
      tabId = await h.panel.open({ machineId: 'local', machineLabel: 'local' });
    });
    const expected = startedSessions[0]?.sessionId;
    expect(expected).toBeTruthy();
    expect(h.panel.getSessionId(tabId)).toBe(expected);
  });

  it('returns null for a tabId that does not exist', async () => {
    const h = mountHarness();
    expect(h.panel.getSessionId('nope')).toBeNull();
  });
});

describe('useTerminalPanel — setTitle() rollback on IPC failure', () => {
  it('keeps the previous title in state when rename_terminal_session rejects', async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === 'list_terminal_sessions') return Promise.resolve([]);
      if (cmd === 'start_terminal_session') return Promise.resolve('sess_1');
      if (cmd === 'rename_terminal_session') return Promise.reject('backend rejected');
      return Promise.resolve(undefined);
    });

    const h = mountHarness();
    let tabId = '';
    await act(async () => {
      tabId = await h.panel.open({
        machineId: 'local',
        machineLabel: 'local',
        repoPath: '/repo',
      });
    });
    expect(h.panel.state.tabs[0]?.title).toBe('repo');

    await act(async () => {
      await expect(h.panel.setTitle(tabId, 'attempted-rename')).rejects.toBe('backend rejected');
    });

    // Title must NOT advance when the backend rejected the rename —
    // otherwise the next list_terminal_sessions reconcile would silently
    // overwrite the UI title with the stale backend value.
    expect(h.panel.state.tabs[0]?.title).toBe('repo');
  });

  it('applies the local title for tabs whose start has not yet resolved', async () => {
    const resolveStart: { current: ((sid: string) => void) | null } = { current: null };
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === 'list_terminal_sessions') return Promise.resolve([]);
      if (cmd === 'start_terminal_session') {
        return new Promise<string>((resolve) => {
          resolveStart.current = resolve;
        });
      }
      return Promise.resolve(undefined);
    });

    const h = mountHarness();
    const openPromise = h.panel.open({
      machineId: 'local',
      machineLabel: 'local',
      repoPath: '/repo',
    });

    await act(async () => {
      for (let i = 0; i < 5; i++) await Promise.resolve();
    });
    const tabId = h.panel.state.tabs[0]!.tabId;

    // Tab is still in `connecting` phase (no sessionId yet) — setTitle
    // should commit the title locally even though the backend IPC has
    // not been issued (no sessionId to address).
    await act(async () => {
      await h.panel.setTitle(tabId, 'early-name');
    });

    expect(h.panel.state.tabs[0]?.title).toBe('early-name');

    // Drain the still-pending start so the test's mock does not leak.
    resolveStart.current?.('sess_x');
    await openPromise.catch(() => {});
  });
});
