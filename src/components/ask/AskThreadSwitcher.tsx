import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { ChevronDown } from 'lucide-react';

import { listAskThreads, EVENT_ASK_TURN_STATUS, type AskTurnStatusPayload } from '../../lib/ask';
import { phaseOfStatus } from '../../lib/askActivity';
import { formatError } from '../../lib/errors';
import { useTauriEvent } from '../../hooks/useTauriEvent';
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

function turnCountLabel(turnCount: number): string {
  return turnCount === 1 ? '1 turn' : `${turnCount} turns`;
}

/**
 * The "Threads ▾" dropdown (`docs/ask-canvas/probe/Empty.html`/`Main.html`'s
 * `.drop` block): every open/closed thread in the project, title + kind chip
 * + turn count per row.
 *
 * **Liveness is read off `ask_turn_status`, never stored.** A thread mid-turn
 * has no column that says so — `AskThread` only knows what the last *settled*
 * turn left behind (`turn_count`, `updated_at`). `DiscoverySection.tsx` faces
 * the identical gap for Discovery cards and solves it the same way: a
 * `Set<string>` of thread ids kept live purely from the event stream,
 * `phaseOfStatus(status) !== null` adding to it and any other status clearing
 * it. Reloading this component drops that set back to empty until the next
 * event arrives, same as Discovery's.
 */
export function AskThreadSwitcher({
  projectId,
  activeThreadId,
  onSelect,
}: AskThreadSwitcherProps): React.ReactElement {
  const [open, setOpen] = useState(false);
  const [threads, setThreads] = useState<AskThread[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [runningThreads, setRunningThreads] = useState<Set<string>>(new Set());

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

  useTauriEvent<AskTurnStatusPayload>(EVENT_ASK_TURN_STATUS, ({ thread_id, status }) => {
    setRunningThreads((prev) => {
      const next = new Set(prev);
      if (phaseOfStatus(status) !== null) next.add(thread_id);
      else next.delete(thread_id);
      return next;
    });
  });

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
