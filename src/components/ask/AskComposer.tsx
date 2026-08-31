import React, { useEffect, useState } from 'react';
import { Send } from 'lucide-react';

import { sendAskTurn } from '../../lib/ask';
import type { TurnPhase } from '../../lib/askActivity';
import { formatError } from '../../lib/errors';
import type { AskMessage } from '../../types';

interface AskComposerProps {
  threadId: string;
  /**
   * Non-null while a turn on this thread is `setting_up`/`working`, per
   * `phaseOfStatus(status)` of the thread's `ask_turn_status` stream — owned
   * by the parent, never re-derived here. The parent clears this to `null`
   * on *every* ending (`success`, `interrupted`, `failed`, `environmental`
   * alike — `discoveryActivity.ts`/`DiscoveryView.tsx`'s handling), and this
   * component never inspects `ending` itself: re-enabling is just "`phase`
   * went back to `null`", so no ending can leave the composer disabled or a
   * bubble hung.
   */
  phase: TurnPhase | null;
  /**
   * Opens/drops this thread's entry in the live-turn store — the write half
   * only. The read/subscribe half (`useStreamedTurn`) belongs solely to
   * `AskStreamingBubble`/`AskCanvasPane`, per `useAskStream.ts`'s own
   * constraint against lifting that subscription.
   */
  begin: (threadId: string, phase: TurnPhase) => void;
  end: (threadId: string) => void;
  /** The user's own message, already persisted — append it to the transcript
   *  before any `ask_agent_event` arrives (Acceptance Criterion 2). */
  onSent: (message: AskMessage) => void;
  /**
   * Seeds the input on mount — a "Try" chip's text, or the empty string.
   * Read once: `AskThreadView` remounts this component (via `key`) to seed a
   * new value rather than pushing updates into an already-mounted composer.
   */
  initialValue?: string;
}

/**
 * Where an Ask turn is typed. Mirrors `InterviewComposer.tsx`'s disabling
 * shape, without the question/attachment machinery Ask has none of.
 *
 * **`sending` outlives the `sendAskTurn` promise on success.** The bubble
 * must stay disabled from the click straight through to the backend's own
 * `setting_up` status, not just through the round trip that persists the
 * user's message — `DiscoveryView.tsx`'s `send` accepts the same gap for the
 * same reason. `sending` only clears once `phase` (the parent's read of that
 * status) turns non-null, or immediately on a rejected send.
 *
 * The typed text follows a different clock: it is cleared only once the
 * message comes back persisted. A rejection — `ALREADY_RUNNING` from a turn
 * running in another window, a closed thread, a dropped IPC — has to leave
 * the question in the box, because nothing else in the app holds a copy of
 * it and the only other feedback is a one-line error.
 */
export function AskComposer({
  threadId,
  phase,
  begin,
  end,
  onSent,
  initialValue,
}: AskComposerProps): React.ReactElement {
  const [value, setValue] = useState(initialValue ?? '');
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (phase !== null) setSending(false);
  }, [phase]);

  const blocked = phase !== null || sending;

  async function send() {
    const text = value.trim();
    if (text.length === 0 || blocked) return;
    setError(null);
    begin(threadId, 'setting_up');
    setSending(true);
    try {
      const message = await sendAskTurn(threadId, text);
      onSent(message);
      setValue('');
    } catch (cause) {
      end(threadId);
      setSending(false);
      setError(formatError(cause));
    }
  }

  return (
    <div className="shrink-0 border-t border-white/5 bg-[#0d0f14]/90 px-4 py-3.5">
      <div className="flex items-end gap-2.5">
        <input
          type="text"
          data-testid="ask-composer"
          aria-label="Ask a question"
          value={value}
          disabled={blocked}
          onChange={(event) => setValue(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== 'Enter') return;
            event.preventDefault();
            void send();
          }}
          placeholder={blocked ? 'A turn is running — this queues behind it…' : 'Ask about the codebase…'}
          className="chat-input disabled:opacity-50"
        />
        <button
          type="button"
          data-testid="ask-composer-send"
          aria-label="Send this turn"
          disabled={blocked || value.trim().length === 0}
          onClick={() => void send()}
          className="btn-primary disabled:cursor-not-allowed disabled:opacity-35"
        >
          <Send className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
      </div>
      {error && (
        <p role="alert" className="mt-2 mb-0 font-mono text-[11px] text-ruby-200">
          {error}
        </p>
      )}
    </div>
  );
}

export default AskComposer;
