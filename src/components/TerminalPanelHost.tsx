import { useCallback, useMemo } from 'react';
import { ChevronDown, TerminalSquare, Trash2, X } from 'lucide-react';

import { useTerminalPanel } from '../hooks/useTerminalPanel';
import { TerminalTab } from './TerminalTab';
import { TerminalSurface } from './TerminalSurface';

export interface TerminalPanelHostProps {
  /**
   * Vertical height of the panel surface in CSS units (vh, px, rem, …).
   * Defaults to `35vh`. Drag-to-resize is intentionally not implemented in
   * V1 (spec §7 Q6).
   */
  height?: string;
}

/**
 * The global, VSCode-style bottom terminal panel. Renders the tab bar,
 * the kill-all button, and the active `TerminalSurface` for the focused
 * tab. The body is hidden via CSS when `state.collapsed` is true so the
 * active surface is NOT unmounted — collapsing must not detach the
 * subscriber on the backend (spec §1 AC #6). Renders nothing when the
 * panel has no tabs.
 */
export function TerminalPanelHost({
  height = '35vh',
}: TerminalPanelHostProps): React.ReactElement | null {
  const { state, close, focus, togglePanel } = useTerminalPanel();

  const activeTab = useMemo(
    () => state.tabs.find((t) => t.tabId === state.activeTabId) ?? null,
    [state.tabs, state.activeTabId],
  );

  const handleCloseTab = useCallback(
    (tabId: string) => {
      void close(tabId);
    },
    [close],
  );

  const handleFocusTab = useCallback(
    (tabId: string) => {
      focus(tabId);
    },
    [focus],
  );

  const handleTogglePanel = useCallback(() => {
    togglePanel();
  }, [togglePanel]);

  const handleKillAll = useCallback(() => {
    for (const tab of state.tabs) {
      void close(tab.tabId);
    }
  }, [state.tabs, close]);

  if (state.tabs.length === 0) {
    return null;
  }

  return (
    <div
      data-testid="terminal-panel-host"
      data-collapsed={state.collapsed ? 'true' : 'false'}
      className="flex flex-col shrink-0 border-t border-white/[0.06] bg-[#08090c]/95 backdrop-blur-md"
      style={{
        height: state.collapsed ? undefined : height,
      }}
    >
      <header
        className="flex items-center gap-1 px-2 py-1.5 border-b border-white/[0.05] bg-[#0d0f14]/80 shrink-0"
        data-testid="terminal-panel-tabbar"
      >
        <div className="flex items-center gap-1 flex-1 min-w-0 overflow-x-auto">
          <div className="flex items-center gap-1 pr-2 mr-1 border-r border-white/[0.05] shrink-0">
            <TerminalSquare className="w-3.5 h-3.5 text-cyan-400" />
            <span className="text-[10px] uppercase tracking-[0.18em] font-bold font-outfit text-slate-400">
              Terminals
            </span>
          </div>
          {state.tabs.map((tab) => (
            <TerminalTab
              key={tab.tabId}
              tab={tab}
              active={tab.tabId === state.activeTabId}
              onFocus={() => handleFocusTab(tab.tabId)}
              onClose={() => handleCloseTab(tab.tabId)}
            />
          ))}
        </div>

        <div className="flex items-center gap-1 pl-2 ml-1 border-l border-white/[0.05] shrink-0">
          <button
            type="button"
            onClick={handleKillAll}
            disabled={state.tabs.length === 0}
            className="p-1.5 rounded-md text-slate-500 hover:text-ruby-300 hover:bg-ruby-500/10 disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
            title="Close all terminal sessions"
            aria-label="Close all terminal sessions"
            data-testid="terminal-panel-kill-all"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </button>
          <button
            type="button"
            onClick={handleTogglePanel}
            className="p-1.5 rounded-md text-slate-500 hover:text-white hover:bg-white/5 transition-colors"
            title={state.collapsed ? 'Show terminal panel' : 'Hide terminal panel'}
            aria-label={state.collapsed ? 'Show terminal panel' : 'Hide terminal panel'}
            data-testid="terminal-panel-hide"
          >
            {state.collapsed ? <ChevronDown className="w-3.5 h-3.5" /> : <X className="w-3.5 h-3.5" />}
          </button>
        </div>
      </header>

      <div
        className={`flex-1 min-h-0 flex flex-col relative ${state.collapsed ? 'hidden' : ''}`}
        data-testid="terminal-panel-body"
        data-visible={state.collapsed ? 'false' : 'true'}
      >
        {activeTab && activeTab.sessionId ? (
          <TerminalSurface
            key={activeTab.sessionId}
            tabId={activeTab.tabId}
            sessionId={activeTab.sessionId}
            phase={activeTab.phase}
            title={activeTab.title}
            machineLabel={activeTab.machineLabel}
          />
        ) : activeTab && activeTab.phase === 'error' ? (
          <div
            className="flex-1 flex flex-col items-center justify-center gap-2 text-xs text-ruby-300 font-mono"
            data-testid="terminal-panel-error"
          >
            <span className="text-[11px] uppercase tracking-[0.18em] font-bold text-ruby-400">
              Terminal failed to start
            </span>
            <span>{activeTab.machineLabel}</span>
            <span className="text-slate-500 text-[10px]">
              Close this tab and try again — check the host reachability if it persists.
            </span>
          </div>
        ) : activeTab ? (
          <div
            className="flex-1 flex items-center justify-center gap-2 text-xs text-slate-500 font-mono"
            data-testid="terminal-panel-connecting"
          >
            <span className="w-1.5 h-1.5 rounded-full bg-amber-400 animate-pulse" />
            <span>connecting to {activeTab.machineLabel}…</span>
          </div>
        ) : (
          <div
            className="flex-1 flex items-center justify-center text-xs text-slate-500 font-mono"
            data-testid="terminal-panel-empty"
          >
            no terminal selected
          </div>
        )}
      </div>
    </div>
  );
}