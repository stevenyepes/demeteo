import React from 'react';

import { TONE_TEXT, type RunStatusTone } from '../../lib/runStatus';

/** `slate` is missing on purpose: a row exists to offer a press, and slate is
 *  what this vocabulary reserves for something inert. */
export type ActionRowTone = Exclude<RunStatusTone, 'slate'>;

/**
 * One offered action: what it does, why, and the button that does it.
 *
 * Shared rather than copied because the Sync pane offers the same shape as the
 * node inspector's Actions tab, and five spellings of a row is four that will
 * not be updated together.
 *
 * The tone classes are held in maps, which is the one place
 * `scripts/check-classes.mjs` cannot see (it reads `className` attributes
 * only). Every value here has been resolved against the compiled stylesheet by
 * hand; adding a tone means doing that again rather than assuming Tailwind
 * derives it.
 */
const ACTION_TONE: Record<ActionRowTone, string> = {
  ruby: 'border-rose-500/20 bg-rose-950/10',
  cyan: 'border-cyan-500/20 bg-cyan-950/10',
  amber: 'border-amber-500/20 bg-amber-950/10',
  violet: 'border-violet-500/20 bg-violet-950/10',
  emerald: 'border-emerald-500/20 bg-emerald-950/10',
};

const ACTION_BTN: Record<ActionRowTone, string> = {
  ruby: 'bg-rose-600 hover:bg-rose-500 text-white',
  cyan: 'bg-cyan-600 hover:bg-cyan-500 text-white',
  amber: 'bg-amber-500 hover:bg-amber-600 text-black',
  violet: 'bg-violet-600 hover:bg-violet-500 text-white',
  emerald: 'bg-emerald-600 hover:bg-emerald-500 text-white',
};

export interface ActionRowProps {
  icon: React.ReactNode;
  tone: ActionRowTone;
  title: string;
  desc: string;
  buttonLabel: string;
  onClick: () => void;
  disabled?: boolean;
  disabledReason?: string;
  /** Hover text for the button when it is live. */
  buttonTitle?: string;
}

export function ActionRow({
  icon,
  tone,
  title,
  desc,
  buttonLabel,
  onClick,
  disabled,
  disabledReason,
  buttonTitle,
}: ActionRowProps) {
  return (
    <div
      data-testid="action-row"
      data-tone={tone}
      className={`flex items-center gap-3 rounded-xl border p-3.5 ${ACTION_TONE[tone]}`}
    >
      <div className={`shrink-0 ${TONE_TEXT[tone]}`}>{icon}</div>
      <div className="min-w-0 flex-1">
        <div className="text-xs font-bold uppercase tracking-wider text-slate-200">{title}</div>
        <div className="mt-0.5 text-[11px] leading-relaxed text-slate-400">{desc}</div>
      </div>
      <button
        type="button"
        onClick={onClick}
        disabled={disabled}
        title={disabled ? disabledReason : buttonTitle}
        className={`shrink-0 rounded-lg px-3 py-1.5 text-xs font-bold transition disabled:cursor-not-allowed disabled:bg-slate-700/40 disabled:text-slate-500 ${ACTION_BTN[tone]}`}
      >
        {buttonLabel}
      </button>
    </div>
  );
}

export default ActionRow;
