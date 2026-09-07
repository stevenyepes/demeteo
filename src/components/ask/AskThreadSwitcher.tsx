import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { ChevronDown } from 'lucide-react';

import { listAskThreads } from '../../lib/ask';
import { turnCountLabel } from '../../lib/askLifecycle';
import { formatError } from '../../lib/errors';
import { useLiveAskTurns } from '../../hooks/useLiveAskTurns';
import { relativeTime } from '../../lib/utils';
import { Chip } from '../ui/Chip';
import type { AskThread } from '../../types';

interface AskThreadSwitcherProps {
  projectId: string;
  /** Highlights the row for the thread already open, mirroring `.drop-row.on`
   *  in `docs/ask-canvas/probe/Empty.html`'s `.drop` block. */
  activeThreadId: string | null;
  /** This component never navigates itself — the caller decides what opening
   *  a thread means. */
  onSelect: (threadId: string) => void;
}

/**
 * The "Threads ▾" dropdown (`docs/ask-canvas/probe/Empty.html`/`Main.html`'s
 * `.drop` block): every open/closed thread in the project, title + kind chip
 * + turn count per row.
 *
 * **Liveness is read off `ask_turn_status`, never stored** — see
 * [`useLiveAskTurns`](../../hooks/useLiveAskTurns.ts), which this shares with
 * `AskSection.tsx` and which records why.
 */
export function AskThreadSwitcher({
  projectId,
  activeThreadId,
  onSelect,
}: AskThreadSwitcherProps): React.ReactElement {
  const [open, setOpen] = useState(false);
  const [threads, setThreads] = useState<AskThread[]>([]);
  const [error, setError] = useState<string | null>(null);
  const runningThreads = useLiveAskTurns();

  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuId = useId();

  const load = useCallback(async () => {
    try {
      const list = await listAskThreads(projectId);
      setThreads(list);
      setError(null);
    } catch (cause) {
      setError(formatError(cause));
    }
  }, [projectId]);

  useEffect(() => {
    void load();
  }, [load]);

  const closeAndRestoreFocus = useCallback(() => {
    setOpen(false);
    triggerRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      closeAndRestoreFocus();
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [open, closeAndRestoreFocus]);

  const toggle = () => {
    setOpen((next) => {
      if (!next) void load();
      return !next;
    });
  };

  return (
    <div ref={containerRef} className="relative shrink-0">
      <button
        type="button"
        ref={triggerRef}
        data-testid="ask-thread-switcher-trigger"
        onClick={toggle}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        className="btn-secondary flex items-center gap-2 text-[13px]"
      >
        Threads
        <Chip size="sm" tone="slate">
          {threads.length}
        </Chip>
        <ChevronDown className="h-3.5 w-3.5" aria-hidden="true" />
      </button>

      {open && (
        <div
          data-testid="ask-thread-switcher-menu"
          className="glass-panel absolute right-0 top-full z-10 mt-2 flex w-[340px] flex-col gap-1 rounded-xl border border-white/10 p-2 shadow-2xl"
        >
          <div className="px-2.5 pt-1.5 pb-2 font-mono text-[10px] font-semibold tracking-widest text-slate-500 uppercase">
            Threads
          </div>
          <div id={menuId} role="menu">
            {error && (
              <p role="alert" className="px-2.5 pb-2 font-mono text-[11px] text-ruby-200">
                {error}
              </p>
            )}
            {!error && threads.length === 0 && (
              <p className="px-2.5 pb-2 text-[12px] text-slate-500">No threads yet.</p>
            )}
            {threads.map((thread) => {
              const live = runningThreads.has(thread.id);
              return (
                <button
                  key={thread.id}
                  type="button"
                  role="menuitem"
                  data-testid="ask-thread-switcher-row"
                  data-active={thread.id === activeThreadId}
                  onClick={() => {
                    closeAndRestoreFocus();
                    onSelect(thread.id);
                  }}
                  className={`flex flex-col gap-1 rounded-lg border px-2.5 py-2 text-left ${
                    thread.id === activeThreadId
                      ? 'border-cyan-500/30 bg-cyan-500/10'
                      : 'border-transparent hover:bg-white/5'
                  }`}
                >
                  <span className="truncate text-[13px] text-slate-100">{thread.title}</span>
                  <span className="flex items-center gap-2 font-mono text-[10px] text-slate-500">
                    <Chip size="sm" tone="cyan">
                      {thread.agent_kind}
                    </Chip>
                    {live && (
                      <Chip size="sm" tone="emerald" dot pulse>
                        live
                      </Chip>
                    )}
                    <span>
                      {relativeTime(thread.updated_at)} · {turnCountLabel(thread.turn_count)}
                    </span>
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

export default AskThreadSwitcher;
