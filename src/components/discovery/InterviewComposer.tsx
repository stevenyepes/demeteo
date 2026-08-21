import React from 'react';
import { Send } from 'lucide-react';

interface InterviewComposerProps {
  /** An open question exists and nothing is streaming. */
  awaiting: boolean;
  pending: boolean;
  disabled: boolean;
  value: string;
  onChange: (value: string) => void;
  onSend: () => void;
  inputRef: React.RefObject<HTMLInputElement | null>;
}

/**
 * Where a turn is typed.
 *
 * The placeholder flips with the interview's state because the two states ask
 * for different things: one settles the question above, the other opens a new
 * thread. Neither is the lesser — `DISCOVERY_UI_SPEC.md` §6.7 keeps the hint
 * saying that both settle the same question.
 *
 * There is no attachment control. `discovery_send_turn` carries text and
 * nothing else (`docs/TASKS_DISCOVERY.md` "Phase 2b"), so a paperclip here
 * would take a file the backend has nowhere to put.
 */
export function InterviewComposer({
  awaiting,
  pending,
  disabled,
  value,
  onChange,
  onSend,
  inputRef,
}: InterviewComposerProps): React.ReactElement {
  const blocked = disabled || pending;

  return (
    <div className="shrink-0 border-t border-white/5 bg-[#0d0f14]/90 px-4 py-3.5">
      {awaiting && (
        <p className="m-0 mb-2 text-[11px] text-slate-500">
          Pick an option above, or answer here — both settle the same question.
        </p>
      )}
      <div className="flex items-end gap-2.5">
        <input
          ref={inputRef}
          type="text"
          data-testid="interview-composer"
          aria-label="Answer the interviewer"
          value={value}
          disabled={blocked}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== 'Enter') return;
            event.preventDefault();
            onSend();
          }}
          placeholder={awaiting ? 'Answer in your own words...' : 'Ask, answer, or push back...'}
          className="chat-input disabled:opacity-50"
        />
        <button
          type="button"
          data-testid="interview-send"
          aria-label="Send this turn"
          disabled={blocked || value.trim().length === 0}
          onClick={onSend}
          className="btn-primary disabled:cursor-not-allowed disabled:opacity-35"
        >
          <Send className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}

export default InterviewComposer;
