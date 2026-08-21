import React, { useEffect } from 'react';
import { Check } from 'lucide-react';

import { optionForKey, type QuestionAnswer } from '../../lib/discoveryInterview';
import type { DiscoveryQuestion, QuestionOption } from '../../types';
import { Chip } from '../ui/Chip';

interface QuestionCardProps {
  question: DiscoveryQuestion;
  /** `null` while this is the open question. */
  answer: QuestionAnswer | null;
  /** The one question the interview is waiting on. */
  live: boolean;
  /** A turn is streaming — clicks and keys are ignored until it lands. */
  pending: boolean;
  onPick: (option: QuestionOption) => void;
  /** Puts the caret in the composer and submits nothing. */
  onAnswerInOwnWords: () => void;
}

/**
 * The interview's primary affordance: a question, its options, and the
 * standing invitation to ignore all of them.
 *
 * **Answering in your own words is first-class.** `DISCOVERY_UI_SPEC.md` §6.7
 * forbids tightening the copy that says so, and the free-text row is an option
 * among the others rather than a link under them — it just submits nothing,
 * because what the user types is the answer and the composer is where they
 * type it.
 *
 * The number keycaps are wired. §6.6 records that the mock draws `1`/`2`/`3`
 * and handles none of them, which is the shape of a promise a surface does not
 * carry.
 */
export function QuestionCard({
  question,
  answer,
  live,
  pending,
  onPick,
  onAnswerInOwnWords,
}: QuestionCardProps): React.ReactElement {
  const answerable = live && !pending;

  useEffect(() => {
    if (!answerable) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      const target = event.target as HTMLElement | null;
      const tag = target?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || target?.isContentEditable) return;
      const id = optionForKey(event.key, question);
      if (id === null) return;
      const option = question.options.find((candidate) => candidate.id === id);
      if (!option) return;
      event.preventDefault();
      onPick(option);
    }
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [answerable, question, onPick]);

  const settled = answer !== null;

  return (
    <div
      data-testid="question-card"
      data-settled={settled}
      className={`w-full rounded-xl border px-4 py-3.5 ${
        settled ? 'border-white/[0.06] bg-white/[0.02]' : 'border-violet-500/20 bg-violet-500/5'
      }`}
    >
      <div className="mb-2.5 flex items-start justify-between gap-2.5">
        <div className="flex min-w-0 items-center gap-1.5 font-mono text-[11px] font-semibold uppercase tracking-wider text-violet-400">
          <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-current" />
          Interviewer
          {question.header && (
            <Chip size="sm" tone="violet" className="ml-1">
              {question.header}
            </Chip>
          )}
        </div>
        {settled ? (
          <Chip size="sm" tone="emerald">
            Answered
          </Chip>
        ) : (
          <Chip size="sm" tone="violet">
            Needs you
          </Chip>
        )}
      </div>

      <p className="m-0 mb-3 text-[13.5px] leading-relaxed text-slate-100">{question.text}</p>

      <div className="flex flex-col gap-2">
        {question.options.map((option, index) => (
          <OptionButton
            key={option.id}
            option={option}
            keycap={String(index + 1)}
            recommended={question.recommended === option.id}
            chosen={answer?.optionId === option.id}
            settled={settled}
            answerable={answerable}
            onClick={() => onPick(option)}
          />
        ))}

        {!settled && (
          <button
            type="button"
            data-testid="question-free-text"
            disabled={!answerable}
            onClick={onAnswerInOwnWords}
            className={`flex w-full items-start gap-2.5 rounded-lg border border-white/[0.06] bg-black/50 px-3 py-2.5 text-left transition ${
              answerable
                ? 'cursor-pointer hover:translate-x-0.5 hover:border-violet-500/45 hover:bg-violet-500/[0.08]'
                : 'cursor-not-allowed'
            }`}
          >
            <Keycap>&crarr;</Keycap>
            <span className="min-w-0">
              <span className="block text-[13px] font-medium text-slate-100">Something else</span>
              <span className="mt-1 block text-[11.5px] leading-relaxed text-slate-400">
                Answer in your own words below. The interviewer takes it as written rather than
                fitting it to the nearest option.
              </span>
            </span>
          </button>
        )}
      </div>

      {answer && answer.optionId === null && (
        <div
          data-testid="question-custom-answer"
          className="mt-2.5 rounded-lg border border-cyan-500/20 bg-cyan-500/[0.06] px-3 py-2.5"
        >
          <div className="mb-1 flex items-center gap-1.5 font-mono text-[11px] font-semibold uppercase tracking-wider text-cyan-400">
            <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-current" />
            You answered in your own words
          </div>
          <p className="m-0 whitespace-pre-wrap text-[12.5px] leading-relaxed text-slate-200">
            {answer.text}
          </p>
        </div>
      )}
    </div>
  );
}

interface OptionButtonProps {
  option: QuestionOption;
  keycap: string;
  recommended: boolean;
  chosen: boolean;
  settled: boolean;
  answerable: boolean;
  onClick: () => void;
}

function OptionButton({
  option,
  keycap,
  recommended,
  chosen,
  settled,
  answerable,
  onClick,
}: OptionButtonProps): React.ReactElement {
  const treatment = chosen
    ? 'border-emerald-500/45 bg-emerald-500/[0.06]'
    : settled
      ? `opacity-[0.38] ${recommended ? 'border-emerald-500/20' : 'border-white/[0.06]'} bg-black/50`
      : `bg-black/50 ${recommended ? 'border-emerald-500/20' : 'border-white/[0.06]'}`;

  return (
    <button
      type="button"
      data-testid="question-option"
      data-option={option.id}
      disabled={!answerable}
      aria-pressed={settled ? chosen : undefined}
      onClick={onClick}
      className={`flex w-full items-start gap-2.5 rounded-lg border px-3 py-2.5 text-left transition ${treatment} ${
        answerable
          ? 'cursor-pointer hover:translate-x-0.5 hover:border-violet-500/45 hover:bg-violet-500/[0.08]'
          : 'cursor-default'
      }`}
    >
      <Keycap chosen={chosen}>{keycap}</Keycap>
      <span className="min-w-0">
        <span className="flex flex-wrap items-center gap-2">
          <span className="text-[13px] font-medium text-slate-100">{option.label}</span>
          {chosen ? (
            <Chip size="sm" tone="emerald" icon={<Check className="h-2.5 w-2.5" />}>
              Chosen
            </Chip>
          ) : (
            recommended &&
            !settled && (
              <Chip size="sm" tone="emerald">
                Recommended
              </Chip>
            )
          )}
        </span>
        <span className="mt-1 block text-[11.5px] leading-relaxed text-slate-400">
          {option.description}
        </span>
      </span>
    </button>
  );
}

function Keycap({
  children,
  chosen = false,
}: {
  children: React.ReactNode;
  chosen?: boolean;
}): React.ReactElement {
  return (
    <span
      aria-hidden="true"
      className={`mt-px flex h-[19px] w-[19px] shrink-0 items-center justify-center rounded border font-mono text-[10px] ${
        chosen ? 'border-emerald-500/45 text-emerald-400' : 'border-white/[0.12] text-slate-400'
      }`}
    >
      {children}
    </span>
  );
}

export default QuestionCard;
