import React, { useCallback, type MouseEvent } from 'react';
import { X } from 'lucide-react';

import type { TerminalTabDescriptor } from '../types';
import { MachineDot } from './ui/MachineDot';
import { PhaseBadge } from './ui/PhaseBadge';
import { AgentBadge } from './ui/AgentBadge';
import { useInlineRename } from '../hooks/useInlineRename';

export interface SessionRowProps {
  tab: TerminalTabDescriptor;
  active: boolean;
  /** Responsive icon-only variant (narrow view) — hides title + badge. */
  collapsed?: boolean;
  onFocus: () => void;
  onClose: () => void;
  onRename: (title: string) => void;
}

/**
 * One row in the Terminals view's vertical session list (spec §4.2, §5).
 * A machine dot + inline-renamable title + phase badge + hover close.
 * Presentational: it owns no session lifecycle, only reports intent up.
 * `React.memo`'d so a focus change re-renders exactly the two affected
 * rows (spec §8).
 */
function SessionRowImpl({
  tab,
  active,
  collapsed = false,
  onFocus,
  onClose,
  onRename,
}: SessionRowProps): React.ReactElement {
  const rename = useInlineRename({ value: tab.title, onCommit: onRename });

  const handleDoubleClick = useCallback(
    (e: MouseEvent<HTMLSpanElement>) => {
      e.stopPropagation();
      rename.startRename();
    },
    [rename],
  );

  const handleCloseClick = useCallback(
    (e: MouseEvent<HTMLButtonElement>) => {
      e.stopPropagation();
      onClose();
    },
    [onClose],
  );

  const pulse = tab.phase === 'running' || tab.phase === 'connecting';

  return (
    <div
      role="tab"
      id={`terminal-tab-${tab.tabId}`}
      aria-selected={active}
      data-testid={`session-row-${tab.tabId}`}
      data-active={active ? 'true' : 'false'}
      onClick={onFocus}
      title={`${tab.title} — ${tab.machineLabel}${tab.repoPath ? ` (${tab.repoPath})` : ''}`}
      className={[
        'group relative flex items-center gap-2 px-3 py-2 cursor-pointer transition-colors border-l-2',
        collapsed ? 'justify-center' : '',
        active
          ? 'border-cyan-400 bg-white/[0.07] text-white'
          : 'border-transparent text-slate-400 hover:bg-white/[0.04] hover:text-slate-200',
      ].join(' ')}
    >
      <MachineDot
        machineId={tab.machineId}
        machineLabel={tab.machineLabel}
        pulse={pulse}
      />

      {!collapsed && (
        <>
          {/* Title (line 1) + machine name (line 2). Two lines keep the
              machine label always visible without stealing width from the
              title, so remote terminals read as "prod-gpu" at a glance. */}
          <div className="flex-1 min-w-0 flex flex-col">
            <div className="flex items-center gap-1.5 min-w-0">
              {rename.renaming ? (
                <input
                  ref={rename.inputRef}
                  type="text"
                  value={rename.draft}
                  onChange={(e) => rename.setDraft(e.target.value)}
                  onBlur={rename.commitRename}
                  onKeyDown={rename.handleKeyDown}
                  onClick={(e) => e.stopPropagation()}
                  onDoubleClick={(e) => e.stopPropagation()}
                  maxLength={rename.maxLength}
                  className="flex-1 min-w-0 bg-[#0d0f14] border border-cyan-500/40 rounded px-1.5 py-0.5 text-xs font-mono text-white outline-none focus:border-cyan-400"
                  data-testid={`session-row-rename-${tab.tabId}`}
                  aria-label="Rename terminal session"
                />
              ) : (
                <span
                  className="min-w-0 truncate text-xs font-mono"
                  onDoubleClick={handleDoubleClick}
                >
                  {tab.title}
                </span>
              )}
              <AgentBadge agentKind={tab.agentKind} className="shrink-0" />
            </div>
            <span
              className="truncate text-[9px] font-mono text-slate-500 leading-tight"
              data-testid={`session-row-machine-${tab.tabId}`}
            >
              {tab.machineLabel}
            </span>
          </div>

          <PhaseBadge phase={tab.phase} className="shrink-0" />

          <button
            type="button"
            onClick={handleCloseClick}
            className="shrink-0 p-0.5 rounded text-slate-500 opacity-0 group-hover:opacity-100 hover:text-white hover:bg-white/10 transition"
            title="Close terminal"
            aria-label={`Close terminal ${tab.title}`}
            data-testid={`session-row-close-${tab.tabId}`}
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </>
      )}
    </div>
  );
}

export const SessionRow = React.memo(SessionRowImpl);
