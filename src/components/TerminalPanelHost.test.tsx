import { act, render, screen } from '@testing-library/react';
import { useEffect, useState, type ReactElement } from 'react';
import { describe, expect, it } from 'vitest';

import { TerminalPanelHost } from './TerminalPanelHost';
import { TerminalPanelContext, type TerminalPanelContextValue } from '../context/TerminalPanelProvider';
import type {
  TerminalPanelState,
  TerminalTabDescriptor,
} from '../types';

function makeTab(id: string, machineId: string, sessionId: string): TerminalTabDescriptor {
  return {
    tabId: id,
    sessionId,
    machineId,
    machineLabel: machineId,
    title: `tab-${id}`,
    phase: 'running',
    createdAt: 1000,
  };
}

function StubHost({
  initial,
  controlRef,
}: {
  initial: TerminalPanelState;
  controlRef: { current: ((next: TerminalPanelState) => void) | null };
}): ReactElement {
  const [state, setState] = useState<TerminalPanelState>(initial);

  useEffect(() => {
    controlRef.current = setState;
  }, [controlRef]);

  const api: TerminalPanelContextValue = {
    state,
    open: async () => '',
    close: async (tabId: string) => {
      setState((prev) => {
        const tabs = prev.tabs.filter((t) => t.tabId !== tabId);
        const activeTabId =
          prev.activeTabId === tabId
            ? tabs.length > 0
              ? tabs[tabs.length - 1].tabId
              : null
            : prev.activeTabId;
        return { ...prev, tabs, activeTabId };
      });
    },
    focus: (tabId: string) => {
      setState((prev) => ({ ...prev, activeTabId: tabId, collapsed: false }));
    },
    setTitle: async () => undefined,
    togglePanel: () => {
      setState((prev) => ({ ...prev, collapsed: !prev.collapsed }));
    },
    getSessionId: () => null,
    consumeStartupReplay: () => null,
  };

  return (
    <TerminalPanelContext.Provider value={api}>
      <TerminalPanelHost />
    </TerminalPanelContext.Provider>
  );
}

function mountHost(initial: TerminalPanelState) {
  const controlRef: { current: ((next: TerminalPanelState) => void) | null } = { current: null };
  const view = render(<StubHost initial={initial} controlRef={controlRef} />);
  return {
    setState: (next: TerminalPanelState) => {
      if (!controlRef.current) {
        throw new Error('StubHost did not mount before setState was called');
      }
      act(() => {
        controlRef.current!(next);
      });
    },
    unmount: () => view.unmount(),
  };
}

describe('TerminalPanelHost — surface per tab', () => {
  it('renders one TerminalSurface for the active tab when expanded', () => {
    mountHost({
      tabs: [
        makeTab('a', 'local', 'sess_a'),
        makeTab('b', 'local', 'sess_b'),
      ],
      activeTabId: 'a',
      collapsed: false,
    });

    expect(screen.getByTestId('terminal-tab-a')).toBeInTheDocument();
    expect(screen.getByTestId('terminal-tab-b')).toBeInTheDocument();
    expect(screen.getAllByTestId('terminal-surface')).toHaveLength(1);
    const surface = screen.getByTestId('terminal-surface');
    expect(surface.getAttribute('data-session-id')).toBe('sess_a');
  });

  it('renders a different TerminalSurface when the active tab changes', () => {
    const harness = mountHost({
      tabs: [
        makeTab('a', 'local', 'sess_a'),
        makeTab('b', 'local', 'sess_b'),
      ],
      activeTabId: 'a',
      collapsed: false,
    });

    expect(
      screen.getByTestId('terminal-surface').getAttribute('data-session-id'),
    ).toBe('sess_a');

    harness.setState({
      tabs: [
        makeTab('a', 'local', 'sess_a'),
        makeTab('b', 'local', 'sess_b'),
      ],
      activeTabId: 'b',
      collapsed: false,
    });

    expect(
      screen.getByTestId('terminal-surface').getAttribute('data-session-id'),
    ).toBe('sess_b');
  });
});

// AC #6: collapsing must NOT unmount the surface — the backend session
// stays attached and the xterm scrollback survives a hide/reveal cycle.
// We hide the body via CSS (`display: none`) instead of unmounting it.
describe('TerminalPanelHost — collapse keeps the surface mounted', () => {
  it('keeps the TerminalSurface mounted but hides the body when collapsed', () => {
    mountHost({
      tabs: [makeTab('a', 'local', 'sess_a')],
      activeTabId: 'a',
      collapsed: true,
    });

    // The surface is still in the DOM (so React never unmounted the
    // TerminalSurface, which is what would have called detach).
    expect(screen.getByTestId('terminal-surface')).toBeInTheDocument();
    // The body wrapper reports visibility=false.
    const body = screen.getByTestId('terminal-panel-body');
    expect(body.getAttribute('data-visible')).toBe('false');
    // And the wrapper carries the `hidden` Tailwind class
    // (display:none). This is the CSS gate that lets us hide without
    // triggering a React unmount.
    expect(body.className).toContain('hidden');
    const host = screen.getByTestId('terminal-panel-host');
    expect(host.getAttribute('data-collapsed')).toBe('true');
  });

  it('un-hides the body when collapsed flips back to false', () => {
    const harness = mountHost({
      tabs: [makeTab('a', 'local', 'sess_a')],
      activeTabId: 'a',
      collapsed: true,
    });

    expect(
      screen.getByTestId('terminal-panel-body').getAttribute('data-visible'),
    ).toBe('false');

    harness.setState({
      tabs: [makeTab('a', 'local', 'sess_a')],
      activeTabId: 'a',
      collapsed: false,
    });

    expect(screen.getByTestId('terminal-surface')).toBeInTheDocument();
    expect(
      screen.getByTestId('terminal-panel-body').getAttribute('data-visible'),
    ).toBe('true');
    expect(screen.getByTestId('terminal-panel-host').getAttribute('data-collapsed')).toBe('false');
  });
});

describe('TerminalPanelHost — empty', () => {
  it('renders nothing when there are no tabs', () => {
    mountHost({
      tabs: [],
      activeTabId: null,
      collapsed: false,
    });

    expect(screen.queryByTestId('terminal-panel-host')).not.toBeInTheDocument();
  });
});

describe('TerminalPanelHost — tab strip', () => {
  it('renders a close button on every tab plus the kill-all affordance', () => {
    mountHost({
      tabs: [
        makeTab('a', 'local', 'sess_a'),
        makeTab('b', 'local', 'sess_b'),
      ],
      activeTabId: 'a',
      collapsed: false,
    });

    expect(screen.getByTestId('terminal-tab-close-a')).toBeInTheDocument();
    expect(screen.getByTestId('terminal-tab-close-b')).toBeInTheDocument();
    expect(screen.getByTestId('terminal-panel-kill-all')).toBeInTheDocument();
  });
});

describe('TerminalPanelHost — error phase', () => {
  it('renders the error overlay when the active tab has phase=error', () => {
    mountHost({
      tabs: [
        {
          tabId: 'a',
          sessionId: null,
          machineId: 'local',
          machineLabel: 'local',
          title: 'broken',
          phase: 'error',
          createdAt: 1000,
        },
      ],
      activeTabId: 'a',
      collapsed: false,
    });

    expect(screen.getByTestId('terminal-panel-error')).toBeInTheDocument();
    expect(screen.queryByTestId('terminal-panel-connecting')).not.toBeInTheDocument();
    expect(screen.queryByTestId('terminal-surface')).not.toBeInTheDocument();
  });

  it('still renders the connecting overlay for tabs still mid-start', () => {
    mountHost({
      tabs: [
        {
          tabId: 'a',
          sessionId: null,
          machineId: 'local',
          machineLabel: 'local',
          title: 'connecting',
          phase: 'connecting',
          createdAt: 1000,
        },
      ],
      activeTabId: 'a',
      collapsed: false,
    });

    expect(screen.getByTestId('terminal-panel-connecting')).toBeInTheDocument();
  });
});
