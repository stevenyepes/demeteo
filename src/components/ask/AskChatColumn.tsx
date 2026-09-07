import React from 'react';
import { Globe, MessageSquare, Network, PanelLeftClose, Route } from 'lucide-react';

import type { TurnPhase } from '../../lib/askActivity';
import type { AskMessage, AskThread, AskThreadDetail } from '../../types';
import { Chip } from '../ui/Chip';
import { AskComposer } from './AskComposer';
import { AskTranscript } from './AskTranscript';
import type { AskStreamStore } from './useAskStream';

interface AskChatColumnProps {
  thread: AskThread | null;
  detail: AskThreadDetail | null;
  projectName: string;
  phase: TurnPhase | null;
  store: AskStreamStore;
  begin: (threadId: string, phase: TurnPhase) => void;
  end: (threadId: string) => void;
  onSent: (message: AskMessage) => void;
  onPickChip: (text: string) => void;
  /** The composer's seed and the nonce that remounts it — the composer owns
   *  its own draft and reads an initial value only on mount. */
  seed: { text: string; nonce: number } | null;
  /** Hidden by class, never unmounted: an unsent draft and the transcript's
   *  scroll position have to survive a hide, the same rule
   *  `InterviewColumn`'s own `hidden` prop follows. */
  hidden?: boolean;
  /** Offered only where hiding this column leaves something else on screen —
   *  with no thread open the canvas pane is an empty placeholder, so the
   *  caller passes nothing and no button renders. */
  onHide?: () => void;
}

/** `Empty.html`'s three `.try` chips, verbatim (Acceptance Criterion 7). Never
 *  a fourth, and never one naming a specific run or ticket. */
function tryChips(resolvedProjectName: string): ReadonlyArray<{
  icon: React.ReactElement;
  tone: 'violet' | 'cyan' | 'emerald';
  text: React.ReactNode;
  seed: string;
  webChip?: boolean;
}> {
  return [
    {
      icon: <Network className="h-3.5 w-3.5" aria-hidden="true" />,
      tone: 'violet',
      text: (
        <>
          Draw the architecture of{' '}
          <code className="rounded border border-white/10 bg-black/40 px-1 py-0.5 font-mono text-[12px] text-cyan-300">
            {resolvedProjectName}
          </code>
        </>
      ),
      seed: `Draw the architecture of ${resolvedProjectName}`,
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
      text: "What changed in this project's dependencies recently?",
      seed: "What changed in this project's dependencies recently?",
      webChip: true,
    },
  ];
}

/**
 * The Ask workspace's left column: the transcript (or the empty state's "Try"
 * chips before a thread has said anything) over the composer.
 */
export function AskChatColumn({
  thread,
  detail,
  projectName,
  phase,
  store,
  begin,
  end,
  onSent,
  onPickChip,
  seed,
  hidden = false,
  onHide,
}: AskChatColumnProps): React.ReactElement {
  return (
    <section
      data-testid="ask-chat-column"
      aria-hidden={hidden ? 'true' : undefined}
      className={`flex w-[480px] shrink-0 flex-col border-r border-white/5 bg-[rgba(11,13,18,0.4)] ${
        hidden ? 'hidden' : ''
      }`}
    >
      {onHide && (
        <div className="flex h-[38px] shrink-0 items-center justify-between gap-3 border-b border-white/5 bg-[#12161e]/60 px-4">
          <span className="truncate font-heading text-xs font-medium text-slate-400">Chat</span>
          <button
            type="button"
            onClick={onHide}
            data-testid="ask-chat-hide"
            aria-label="Hide the chat"
            title="Hide the chat — the turn keeps running"
            className="rounded p-1 text-slate-400 transition-colors hover:bg-white/5 hover:text-white"
          >
            <PanelLeftClose className="h-3.5 w-3.5" />
          </button>
        </div>
      )}

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
            {tryChips(projectName).map((chip) => (
              <button
                key={chip.seed}
                type="button"
                data-testid="ask-try-chip"
                onClick={() => onPickChip(chip.seed)}
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
          onSent={onSent}
          initialValue={seed?.text}
          closed={thread.status === 'closed'}
        />
      )}
    </section>
  );
}

export default AskChatColumn;
