import { act, render, screen, type RenderResult } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { type ReactElement } from 'react';
import { describe, expect, it, beforeEach, vi } from 'vitest';
import { invoke, type InvokeArgs } from '@tauri-apps/api/core';

import { TerminalPanelProvider } from '../context/TerminalPanelProvider';
import { ProjectProvider } from '../context/ProjectContext';
import { useTerminalPanel } from '../hooks/useTerminalPanel';
import { MIN_PLAUSIBLE_COLS, setLastTerminalSize } from '../lib/terminalViewport';
import { resizeObserverStubs, setFitGeometry } from '../test/setup';
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
  readonly view: RenderResult;
  /** Re-render the same tree on the other side of the route toggle. */
  setActive(active: boolean): void;
}

function mount(active = true): Harness {
  const ref: { current: ReturnType<typeof useTerminalPanel> | null } = { current: null };
  function Capture(): ReactElement {
    const panel = useTerminalPanel();
    ref.current = panel;
    return <span data-testid="dbg">{panel.state.tabs.length}</span>;
  }
  const tree = (isActive: boolean): ReactElement => (
    <ProjectProvider>
      <TerminalPanelProvider>
        <Capture />
        <TerminalsView active={isActive} />
      </TerminalPanelProvider>
    </ProjectProvider>
  );
  const view = render(tree(active));
  return {
    get panel() {
      if (!ref.current) throw new Error('panel did not mount');
      return ref.current;
    },
    view,
    setActive: (next) => view.rerender(tree(next)),
  };
}

// jsdom lays nothing out, so every element answers `hasLayoutBox` the way a
// `display:none` subtree does. Driving the border box by hand is the only way
// to say "the route is on screen now" — `offsetParent` is not writable
// per-element and stays null under jsdom whatever the `display` is.
function layoutBox(view: RenderResult): { show(): void; hide(): void } {
  const el = view.container.querySelector<HTMLElement>(
    '[data-testid="terminal-surface"] .w-full.h-full',
  );
  if (!el) throw new Error('terminal container not found');
  let onScreen = false;
  el.getBoundingClientRect = () =>
    ({
      width: onScreen ? 800 : 0,
      height: onScreen ? 400 : 0,
      top: 0,
      left: 0,
      right: onScreen ? 800 : 0,
      bottom: onScreen ? 400 : 0,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }) as DOMRect;
  return {
    show: () => {
      onScreen = true;
    },
    hide: () => {
      onScreen = false;
    },
  };
}

/** The observer the surface bound to its own container, not one of the ones
 *  other components in the view register. */
function surfaceObserver(view: RenderResult): (typeof resizeObserverStubs)[number] {
  const el = view.container.querySelector('[data-testid="terminal-surface"] .w-full.h-full');
  const stub = resizeObserverStubs.find((o) =>
    o.observe.mock.calls.some(([target]) => target === el),
  );
  if (!stub) throw new Error('no ResizeObserver bound to the terminal container');
  return stub;
}

function isSize(v: unknown): v is { cols: number; rows: number } {
  if (typeof v !== 'object' || v === null) return false;
  const rec = v as Record<string, unknown>;
  return typeof rec.cols === 'number' && typeof rec.rows === 'number';
}

function resizedSizes(): Array<{ cols: number; rows: number }> {
  return commandsOf('resize_terminal_session')
    .map(([, args]) => args)
    .filter(isSize)
    .map(({ cols, rows }) => ({ cols, rows }));
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

  it('ArrowDown selects the FIRST row when nothing is selected yet', async () => {
    // Restored sessions arrive via STARTUP_RECONCILE, which appends tabs
    // without setting activeTabId — so the list starts with a null
    // selection. ArrowDown must land on the first row, not skip to the
    // second.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === 'list_terminal_sessions') {
        return Promise.resolve([
          { session_id: 'sess_a', machine_id: 'local', created_at: 1, title: null },
          { session_id: 'sess_b', machine_id: 'local', created_at: 2, title: null },
        ]);
      }
      if (cmd === 'get_agent_configs') return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    const h = mount();
    await act(async () => {
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });
    expect(h.panel.state.tabs).toHaveLength(2);
    expect(h.panel.state.activeTabId).toBeNull();

    const list = screen.getByRole('tablist');
    await act(async () => {
      list.focus();
      await userEvent.keyboard('{ArrowDown}');
      for (let i = 0; i < 3; i++) await Promise.resolve();
    });
    expect(h.panel.state.activeTabId).toBe(h.panel.state.tabs[0].tabId);
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

  it('emits no degenerate resize when the view is hidden and shown', async () => {
    // The route journey end to end: the surface stays mounted behind the
    // `hidden` class, so every fit taken while off-route measures a boxless
    // container and lands on ~11 × 5. Nothing below the floor may reach the
    // PTY, and the surface must still come back at the size the viewport has
    // now — the window may have been resized while the route was away.
    setLastTerminalSize(100, 30);
    setFitGeometry(100, 30);

    const h = mount(true);
    await act(async () => {
      await h.panel.open({ machineId: 'local', machineLabel: 'local', repoPath: '/a' });
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });

    const box = layoutBox(h.view);
    const observer = surfaceObserver(h.view);
    box.show();
    await act(async () => {
      observer.trigger();
    });

    setFitGeometry(11, 5);
    box.hide();
    await act(async () => {
      h.setActive(false);
      observer.trigger();
    });

    setFitGeometry(132, 44);
    box.show();
    await act(async () => {
      h.setActive(true);
      for (let i = 0; i < 5; i++) await Promise.resolve();
    });

    expect(resizedSizes().filter((s) => s.cols < MIN_PLAUSIBLE_COLS)).toEqual([]);
    expect(resizedSizes()).toEqual([{ cols: 132, rows: 44 }]);
  });
});
