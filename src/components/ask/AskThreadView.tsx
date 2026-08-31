import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Globe, MessageSquare, Network, Route } from 'lucide-react';

import { useTauriEvent } from '../../hooks/useTauriEvent';
import {
  askTurnRunning,
  EVENT_ASK_TURN_COMPLETED,
  EVENT_ASK_TURN_STATUS,
  listAskThreads,
  loadAskThread,
  type AskTurnCompletedPayload,
  type AskTurnStatusPayload,
} from '../../lib/ask';
import { phaseOfStatus, type TurnPhase } from '../../lib/askActivity';
import { formatError } from '../../lib/errors';
import type { AskMessage, AskThread, AskThreadDetail } from '../../types';
import { Chip } from '../ui/Chip';
import { AskCanvasPane } from './AskCanvasPane';
import { AskComposer } from './AskComposer';
import { AskThreadSettingsPanel } from './AskThreadSettingsPanel';
import { AskThreadSwitcher } from './AskThreadSwitcher';
import { AskTranscript } from './AskTranscript';
import { AskWorkspaceHeader } from './AskWorkspaceHeader';
import { NewAskThreadModal } from './NewAskThreadModal';
import { useAskStream } from './useAskStream';

interface AskThreadViewProps {
  projectId: string;
  /** The project's own host — what `NewAskThreadModal`'s machine picker
   *  starts on, same convention as `DiscoverySection`'s `machineId`. */
  machineId: string;
}

/** `Empty.html`'s three `.try` chips, verbatim (Acceptance Criterion 7). Never
 *  a fourth, and never one naming a specific run or ticket. */
const TRY_CHIPS: ReadonlyArray<{
  icon: React.ReactElement;
  tone: 'violet' | 'cyan' | 'emerald';
  text: React.ReactNode;
  seed: string;
  webChip?: boolean;
}> = [
  {
    icon: <Network className="h-3.5 w-3.5" aria-hidden="true" />,
    tone: 'violet',
    text: (
      <>
        Draw the architecture of{' '}
        <code className="rounded border border-white/10 bg-black/40 px-1 py-0.5 font-mono text-[12px] text-cyan-300">
          crates/demeteo-core
        </code>
      </>
    ),
    seed: 'Draw the architecture of crates/demeteo-core',
  },
  {
    icon: <Route className="h-3.5 w-3.5" aria-hidden="true" />,
    tone: 'cyan',
    text: 'Map the journey from New Feature to a merged branch',
    seed: 'Map the journey from New Feature to a merged branch',
  },
  {
    icon: <Globe className="h-3.5 w-3.5" aria-hidden="true" />,
    tone: 'emerald',
    text: 'What changed in Tauri v2 capabilities since 2.1?',
    seed: 'What changed in Tauri v2 capabilities since 2.1?',
    webChip: true,
  },
];

/**
 * One project's Ask workspace (`docs/ask-canvas/probe/Main.html`/`Empty.html`):
 * the header, the transcript column, and the canvas pane beside it. Thin by
 * construction (AGENTS.md §3) — every column is a component built elsewhere;
 * this file only owns which thread is open and the turn-lifecycle events that
 * decide `phase` (nothing here parses an `AgentEvent` itself).
 *
 * `useAskStream` is instantiated once, here, and only its `store` travels
 * down — the subscription itself stays leaf-mounted in `AskStreamingBubble`/
 * `AskCanvasPane`, per that hook's own doc comment.
 */
export function AskThreadView({ projectId, machineId }: AskThreadViewProps): React.ReactElement {
  const [threads, setThreads] = useState<AskThread[] | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<AskThreadDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [phase, setPhase] = useState<TurnPhase | null>(null);
  // Non-null while `NewAskThreadModal` is up, carrying the name it opens on
  // — a "Try" chip's text when a chip is what opened it.
  const [newThread, setNewThread] = useState<{ seedTitle: string } | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // A chip's text, remounting the composer (by nonce) to seed it — the
  // composer owns its own input state and takes an initial value only once,
  // on mount, per its own doc comment.
  const [seed, setSeed] = useState<{ text: string; nonce: number } | null>(null);

  const { store, begin, end } = useAskStream();
  // Whether the turn stream has spoken about the selected thread yet. The
  // recovery read below is only true as of the moment it was asked, so a turn
  // that ends while it is in flight would otherwise be re-opened by its stale
  // answer and never closed again.
  const streamSpoke = useRef(false);

  const refreshThreads = useCallback(async (): Promise<AskThread[]> => {
    try {
      const list = await listAskThreads(projectId);
      setThreads(list);
      return list;
    } catch (cause) {
      setError(formatError(cause));
      return [];
    }
  }, [projectId]);

  useEffect(() => {
    setThreads(null);
    setSelectedId(null);
    setDetail(null);
    setError(null);
    void refreshThreads().then((list) => {
      const openThread = list.find((t) => t.status === 'open');
      if (openThread) setSelectedId(openThread.id);
    });
  }, [refreshThreads]);

  const loadSelected = useCallback(async () => {
    if (selectedId === null) return;
    try {
      const loaded = await loadAskThread(selectedId);
      setDetail(loaded);
      setError(null);
    } catch (cause) {
      setError(formatError(cause));
    }
  }, [selectedId]);

  useEffect(() => {
    setDetail(null);
    setPhase(null);
    void loadSelected();
  }, [loadSelected]);

  // A turn that was already under way when this thread was selected — a
  // reload, or a navigation away and back, part-way through a long answer.
  // Its opening `ask_turn_status` was emitted before anything here listened
  // and statuses are transitions, never repeated, so without this read the
  // screen sits idle for the rest of the turn and the composer's next Send
  // comes back `ALREADY_RUNNING`. Asked once per selected thread: everything
  // after it arrives on the stream.
  useEffect(() => {
    const threadId = selectedId;
    if (threadId === null) return;
    let dropped = false;
    streamSpoke.current = false;
    void askTurnRunning(threadId)
      .then((running) => {
        if (dropped || streamSpoke.current || !running) return;
        setPhase('working');
        begin(threadId, 'working');
      })
      .catch(() => {
        // Whatever broke this read broke `loadSelected` too, and it reports
        // it — a second banner naming the same cause would say nothing.
      });
    return () => {
      dropped = true;
    };
  }, [selectedId, begin]);

  useTauriEvent<AskTurnStatusPayload>(EVENT_ASK_TURN_STATUS, ({ thread_id, status, reason }) => {
    const next = phaseOfStatus(status);
    // A status that ends the turn is the only end `announced()`'s
    // prepare-failure branch ever emits — it returns before `run()`, so no
    // `ask_turn_completed` follows it. Ending here as well as there is
    // idempotent (`end` no-ops on a thread with no turn) and is what keeps
    // the next turn on this thread from inheriting the failed attempt's
    // clock and ledger.
    if (next === null) end(thread_id);
    else begin(thread_id, next);
    // Above this line the thread the payload names; below it, the thread on
    // screen. The store is keyed by `thread_id` and `useAskStream` folds
    // deltas into it whatever is selected, so hoisting this guard over
    // `begin`/`end` strands the entry of any thread the user walks away from
    // mid-turn — and the next question asked on it renders that turn's text,
    // ledger and clock. `AskThreadSwitcher` folds this event unguarded too.
    if (thread_id !== selectedId) return;
    // Selection-scoped for a second reason: it guards *this* thread's
    // recovery read, which another thread's traffic says nothing about.
    streamSpoke.current = true;
    setPhase(next);
    if (status === 'error' && reason) setError(reason);
  });

  useTauriEvent<AskTurnCompletedPayload>(EVENT_ASK_TURN_COMPLETED, (payload) => {
    end(payload.thread_id);
    if (payload.thread_id !== selectedId) return;
    streamSpoke.current = true;
    setPhase(null);
    void refreshThreads();
    void loadSelected();
    if (payload.ending !== 'success' && payload.reason) setError(payload.reason);
  });

  function selectThread(threadId: string) {
    if (threadId === selectedId) return;
    setSelectedId(threadId);
    setSeed(null);
  }

  function handleSent(message: AskMessage) {
    setDetail((current) =>
      current && current.thread.id === message.thread_id
        ? {
            ...current,
            messages: [...current.messages, { ...message, prose: message.text, canvas: null, canvas_error: null }],
          }
        : current,
    );
  }

  function pickChip(text: string) {
    setSeed((current) => ({ text, nonce: (current?.nonce ?? 0) + 1 }));
    // With no thread open there is no composer to seed yet, and the chips are
    // the whole surface — so the text rides through the modal as the thread's
    // name and lands in the composer that mounts with the created thread.
    if (selectedId === null) setNewThread({ seedTitle: text });
  }

  function closeNewThread() {
    setNewThread(null);
    // A dismissed modal drops the chip's seed with it, but only while no
    // thread is open: the composer's `key` carries the nonce, so clearing the
    // seed under a mounted one remounts it and takes any draft down too.
    if (selectedId === null) setSeed(null);
  }

  if (error && threads === null) {
    return (
      <div className="flex flex-1 items-center justify-center bg-[#0a0c10]">
        <p role="alert" className="font-mono text-xs text-ruby-200">
          {error}
        </p>
      </div>
    );
  }

  const thread = detail?.thread ?? null;

  return (
    <div className="relative flex flex-1 flex-col overflow-hidden bg-[#0a0c10]">
      {thread ? (
        <AskWorkspaceHeader
          thread={thread}
          projectId={projectId}
          onSelectThread={selectThread}
          onNewThread={() => setNewThread({ seedTitle: '' })}
          onOpenSettings={() => setSettingsOpen(true)}
        />
      ) : (
        <header className="flex shrink-0 items-center justify-between gap-6 border-b border-white/5 bg-[#0d0f14]/60 px-6 py-3.5">
          <div className="flex min-w-0 flex-col gap-1.5">
            <p className="m-0 font-mono text-[11px] text-slate-500">Ask</p>
            <h1 className="m-0 font-heading text-xl font-bold tracking-tight text-white">New thread</h1>
          </div>
          <div className="flex shrink-0 items-center gap-4">
            <AskThreadSwitcher projectId={projectId} activeThreadId={null} onSelect={selectThread} />
            <button
              type="button"
              data-testid="ask-new-thread"
              onClick={() => setNewThread({ seedTitle: '' })}
              className="btn-primary inline-flex items-center gap-2"
            >
              New thread
            </button>
          </div>
        </header>
      )}

      {error && (
        <p
          role="alert"
          className="m-0 shrink-0 border-b border-ruby-500/20 bg-ruby-500/5 px-6 py-2 font-mono text-[11px] text-ruby-200"
        >
          {error}
        </p>
      )}

      <div className="flex min-h-0 flex-1">
        <section className="flex w-[480px] shrink-0 flex-col border-r border-white/5 bg-[rgba(11,13,18,0.4)]">
          {thread && detail && detail.messages.length > 0 ? (
            <AskTranscript
              threadId={thread.id}
              messages={detail.messages}
              pending={phase !== null}
              store={store}
            />
          ) : (
            <div className="flex min-h-0 flex-1 flex-col items-center overflow-y-auto px-8 pt-12 text-center">
              <div className="flex h-11 w-11 items-center justify-center rounded-xl border border-violet-500/30 bg-violet-500/10 text-violet-300">
                <MessageSquare className="h-5 w-5" aria-hidden="true" />
              </div>
              <h2 className="mt-4 font-heading text-[17px] font-semibold text-white">
                Ask about this project
              </h2>
              <p className="mt-2 max-w-[340px] text-[13px] leading-relaxed text-slate-400">
                Questions about the code, a run, or the pipeline — the repo, and the web when it needs
                it. Ask for a diagram and it lands on the canvas beside you.
              </p>

              <div className="mt-7 w-full">
                <p className="mb-2.5 self-start font-mono text-[10px] tracking-[0.1em] text-slate-600 uppercase">
                  Try
                </p>
                {TRY_CHIPS.map((chip) => (
                  <button
                    key={chip.seed}
                    type="button"
                    data-testid="ask-try-chip"
                    onClick={() => pickChip(chip.seed)}
                    className="mb-2 flex w-full items-center gap-2.5 rounded-xl border border-white/5 bg-white/[0.03] px-3 py-2.5 text-left"
                  >
                    <span
                      className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border ${
                        chip.tone === 'violet'
                          ? 'border-violet-500/20 bg-violet-500/10 text-violet-300'
                          : chip.tone === 'cyan'
                            ? 'border-cyan-500/20 bg-cyan-500/10 text-cyan-300'
                            : 'border-emerald-500/20 bg-emerald-500/10 text-emerald-300'
                      }`}
                    >
                      {chip.icon}
                    </span>
                    <span className="min-w-0 flex-1 text-[13px] leading-snug text-slate-200">
                      {chip.text}
                    </span>
                    {chip.webChip && (
                      <Chip size="sm" tone="emerald">
                        web
                      </Chip>
                    )}
                  </button>
                ))}
              </div>
            </div>
          )}

          {thread && (
            <AskComposer
              key={`composer-${thread.id}-${seed?.nonce ?? 0}`}
              threadId={thread.id}
              phase={phase}
              begin={begin}
              end={end}
              onSent={handleSent}
              initialValue={seed?.text}
            />
          )}
        </section>

        <section className="flex min-w-0 flex-1 flex-col">
          {thread ? (
            <AskCanvasPane
              store={store}
              threadId={thread.id}
              lastMessage={
                detail && detail.messages.length > 0 ? detail.messages[detail.messages.length - 1] : null
              }
              phase={phase}
            />
          ) : (
            <div
              data-testid="ask-canvas-placeholder"
              className="flex h-full w-full items-center justify-center font-mono text-[11px] text-slate-500"
            >
              No canvas yet.
            </div>
          )}
        </section>
      </div>

      {newThread && (
        <NewAskThreadModal
          projectId={projectId}
          machineId={machineId}
          seedTitle={newThread.seedTitle}
          onClose={closeNewThread}
          onCreated={(created) => {
            setNewThread(null);
            void refreshThreads();
            setSelectedId(created.id);
          }}
        />
      )}

      {settingsOpen && thread && (
        <AskThreadSettingsPanel
          thread={thread}
          onClose={() => setSettingsOpen(false)}
          onSaved={(updated) => {
            setDetail((current) => (current ? { ...current, thread: updated } : current));
            void refreshThreads();
          }}
        />
      )}
    </div>
  );
}

export default AskThreadView;
