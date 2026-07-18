import React, { useCallback } from 'react';
import { RotateCw, TerminalSquare, Trash2 } from 'lucide-react';

import { useTerminalPanel } from '../hooks/useTerminalPanel';
import { TerminalSurface } from './TerminalSurface';
import { SessionRow } from './SessionRow';
import { NewTerminalMenu } from './NewTerminalMenu';
import { ScrollArea } from './ui/ScrollArea';

export interface TerminalsViewProps {
  /** Whether the Terminals route is currently active. The view stays
   *  mounted off-route (so the active xterm survives navigation, spec
   *  §4.1) and toggles visibility with CSS. */
  active: boolean;
}

/**
 * Full-page Terminals view (spec §4). A vertical list of session tabs on
 * the left and a single active `TerminalSurface` on the right. The view
 * is mounted once and hidden off-route rather than unmounted, so the
 * active terminal's xterm — and every backend session — survives
 * navigation (invariant 2, §4.1). Only the active tab mounts a surface
 * (invariant 3).
 */
export function TerminalsView({ active }: TerminalsViewProps): React.ReactElement {
  const { state, focus, close, setTitle, reconnect } = useTerminalPanel();
  const { tabs, activeTabId } = state;
  const activeTab = tabs.find((t) => t.tabId === activeTabId) ?? null;

  const handleRename = useCallback(
    (tabId: string, title: string) => {
      void setTitle(tabId, title);
    },
    [setTitle],
  );

  const handleCloseAll = useCallback(() => {
    for (const t of tabs) void close(t.tabId);
  }, [tabs, close]);

  // Roving ↑/↓ selection within the session list (spec §4.2, §4.3).
  const handleListKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
      if (tabs.length === 0) return;
      e.preventDefault();
      const current = Math.max(
        0,
        tabs.findIndex((t) => t.tabId === activeTabId),
      );
      const next =
        e.key === 'ArrowDown'
          ? Math.min(tabs.length - 1, current + 1)
          : Math.max(0, current - 1);
      const target = tabs[next];
      if (target) focus(target.tabId);
    },
    [tabs, activeTabId, focus],
  );

  return (
    <div
      data-testid="terminals-view"
      aria-hidden={!active}
      className={[
        'absolute inset-0 z-20 bg-[#08090c]',
        active ? 'flex' : 'hidden',
      ].join(' ')}
    >
      {tabs.length === 0 ? (
        <div className="flex-1 flex flex-col items-center justify-center gap-4 text-center">
          <TerminalSquare className="w-10 h-10 text-slate-600" />
          <div className="text-sm text-slate-400 font-mono">No terminals open</div>
          <div className="text-[11px] text-slate-600 font-mono max-w-xs">
            Open a shell or launch a coding agent. Sessions stay alive as you
            navigate around the app.
          </div>
          <NewTerminalMenu />
        </div>
      ) : (
        <>
          {/* Session list */}
          <div className="w-56 shrink-0 border-r border-white/5 bg-[#0b0c10] flex flex-col min-h-0">
            <div className="flex items-center justify-between px-3 py-2 border-b border-white/5 shrink-0">
              <div className="flex items-center gap-2 text-[11px] font-mono text-slate-300">
                <span className="uppercase tracking-wider font-semibold text-slate-400">
                  Terminals
                </span>
                <span className="text-cyan-400">{tabs.length}</span>
              </div>
              <div className="flex items-center gap-1">
                <NewTerminalMenu />
                <button
                  type="button"
                  onClick={handleCloseAll}
                  className="p-1 rounded text-slate-500 hover:text-ruby-400 hover:bg-white/5 transition"
                  title="Close all terminals"
                  aria-label="Close all terminals"
                  data-testid="terminals-close-all"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            </div>

            <ScrollArea
              className="flex-1"
              role="tablist"
              aria-orientation="vertical"
              tabIndex={0}
              onKeyDown={handleListKeyDown}
            >
              {tabs.map((tab) => (
                <SessionRow
                  key={tab.tabId}
                  tab={tab}
                  active={tab.tabId === activeTabId}
                  onFocus={() => focus(tab.tabId)}
                  onClose={() => void close(tab.tabId)}
                  onRename={(title) => handleRename(tab.tabId, title)}
                />
              ))}
            </ScrollArea>
          </div>

          {/* Active surface */}
          <div className="flex-1 min-h-0 relative flex flex-col">
            {activeTab &&
            activeTab.sessionId &&
            (activeTab.phase === 'running' || activeTab.phase === 'disconnected') ? (
              <>
                <TerminalSurface
                  key={activeTab.tabId}
                  tabId={activeTab.tabId}
                  sessionId={activeTab.sessionId}
                  phase={activeTab.phase}
                  title={activeTab.title}
                  machineLabel={activeTab.machineLabel}
                />
                {activeTab.phase === 'disconnected' && (
                  <button
                    type="button"
                    onClick={() => void reconnect(activeTab.tabId)}
                    className="absolute top-2 right-3 z-10 flex items-center gap-1.5 px-2.5 py-1 rounded-md text-[11px] font-mono border border-amber-400/40 bg-amber-400/10 text-amber-300 hover:bg-amber-400/20 transition"
                    title="Reconnect this session"
                    data-testid="terminals-reconnect"
                  >
                    <RotateCw className="w-3.5 h-3.5" />
                    <span>Reconnect</span>
                  </button>
                )}
              </>
            ) : (
              <div className="flex-1 flex items-center justify-center text-[11px] font-mono text-slate-500">
                {activeTab && activeTab.phase === 'connecting'
                  ? 'Connecting…'
                  : activeTab
                    ? 'Session is not running.'
                    : 'Select a terminal.'}
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}
