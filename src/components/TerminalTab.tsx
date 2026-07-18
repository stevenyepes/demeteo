import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
} from 'react';
import { X } from 'lucide-react';

import { useTerminalPanel } from '../hooks/useTerminalPanel';
import type { TerminalTabDescriptor } from '../types';

export interface TerminalTabProps {
  tab: TerminalTabDescriptor;
  active: boolean;
  onFocus: () => void;
  onClose: () => void;
}

const MACHINE_LABEL_LOCAL = 'local';

/**
 * Visual marker for the host machine. Local terminals get a cyan dot
 * (matches the TerminalWindow status palette); remote terminals get an
 * emerald dot so the user can scan the strip at a glance.
 */
function machineDotColor(machineId: string, machineLabel: string): string {
  if (machineId === 'local' || machineLabel.toLowerCase() === MACHINE_LABEL_LOCAL) {
    return 'bg-cyan-400';
  }
  return 'bg-emerald-400';
}

/**
 * A single tab in the global terminal panel. Shows the user title
 * (double-click to rename inline), a coloured machine dot, a running
 * pulse while the session is alive, and a close button.
 */
export function TerminalTab({ tab, active, onFocus, onClose }: TerminalTabProps): React.ReactElement {
  const { setTitle } = useTerminalPanel();
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState(tab.title);
  const inputRef = useRef<HTMLInputElement | null>(null);

  // Sync the draft when the upstream title changes (e.g. another tab's
  // rename IPC round-tripped while we weren't editing).
  useEffect(() => {
    if (!renaming) setDraft(tab.title);
  }, [tab.title, renaming]);

  // Auto-focus the rename input the moment we enter rename mode.
  useEffect(() => {
    if (renaming && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [renaming]);

  const commitRename = useCallback(() => {
    const trimmed = draft.trim();
    setRenaming(false);
    if (trimmed === tab.title) {
      setDraft(tab.title);
      return;
    }
    void setTitle(tab.tabId, trimmed);
  }, [draft, tab.tabId, tab.title, setTitle]);

  const cancelRename = useCallback(() => {
    setDraft(tab.title);
    setRenaming(false);
  }, [tab.title]);

  const handleDoubleClick = useCallback(
    (e: MouseEvent<HTMLSpanElement>) => {
      e.stopPropagation();
      setDraft(tab.title);
      setRenaming(true);
    },
    [tab.title],
  );

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        commitRename();
      } else if (e.key === 'Escape') {
        e.preventDefault();
        cancelRename();
      }
    },
    [commitRename, cancelRename],
  );

  const handleCloseClick = useCallback(
    (e: MouseEvent<HTMLButtonElement>) => {
      e.stopPropagation();
      onClose();
    },
    [onClose],
  );

  const phase = tab.phase;
  const showPulse = phase === 'running' || phase === 'connecting';

  return (
    <div
      role="tab"
      aria-selected={active}
      data-testid={`terminal-tab-${tab.tabId}`}
      data-active={active ? 'true' : 'false'}
      onClick={onFocus}
      onDoubleClick={handleDoubleClick}
      className={[
        'group flex items-center gap-2 px-3 py-1.5 rounded-md cursor-pointer transition-colors shrink-0 max-w-[220px]',
        active
          ? 'bg-white/[0.07] border border-white/10 text-white'
          : 'bg-transparent border border-transparent text-slate-400 hover:bg-white/[0.04] hover:text-slate-200',
      ].join(' ')}
    >
      <span
        aria-hidden="true"
        className={`w-1.5 h-1.5 rounded-full shrink-0 ${machineDotColor(tab.machineId, tab.machineLabel)} ${
          showPulse ? 'animate-pulse-glow' : 'opacity-60'
        }`}
      />

      {renaming ? (
        <input
          ref={inputRef}
          type="text"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commitRename}
          onKeyDown={handleKeyDown}
          onClick={(e) => e.stopPropagation()}
          onDoubleClick={(e) => e.stopPropagation()}
          maxLength={64}
          className="flex-1 min-w-0 bg-[#0d0f14] border border-cyan-500/40 rounded px-1.5 py-0.5 text-xs font-mono text-white outline-none focus:border-cyan-400"
          data-testid={`terminal-tab-rename-${tab.tabId}`}
          aria-label="Rename terminal tab"
        />
      ) : (
        <span
          className="flex-1 min-w-0 truncate text-xs font-mono"
          title={`${tab.title} — ${tab.machineLabel}${tab.repoPath ? ` (${tab.repoPath})` : ''}`}
        >
          {tab.title}
        </span>
      )}

      {phase === 'closed' && (
        <span
          aria-hidden="true"
          className="text-[9px] uppercase tracking-wider text-ruby-400/80 font-bold shrink-0"
        >
          closed
        </span>
      )}
      {phase === 'error' && (
        <span
          aria-hidden="true"
          className="text-[9px] uppercase tracking-wider text-ruby-400 font-bold shrink-0"
        >
          err
        </span>
      )}

      <button
        type="button"
        onClick={handleCloseClick}
        className="shrink-0 p-0.5 rounded text-slate-500 hover:text-white hover:bg-white/10 transition-colors opacity-60 group-hover:opacity-100"
        title={`Close ${tab.title}`}
        aria-label={`Close ${tab.title}`}
        data-testid={`terminal-tab-close-${tab.tabId}`}
      >
        <X className="w-3 h-3" />
      </button>
    </div>
  );
}