import React from 'react';

import { TONE_TEXT, type RunStatusTone } from '../../lib/runStatus';
import { Button } from './Button';

/** `slate` is missing on purpose: a row exists to offer a press, and slate is
 *  what this vocabulary reserves for something inert. A row that announces no
 *  state takes no tone at all and renders neutral. */
export type ActionRowTone = Extract<RunStatusTone, 'ruby' | 'cyan' | 'amber' | 'emerald'>;

/**
 * One offered action: what it does, why, and the button that does it.
 *
 * Shared rather than copied because the Sync pane offers the same shape as the
 * node inspector's Actions tab, and five spellings of a row is four that will
 * not be updated together.
 *
 * The tone tints the row, never the button: the button is the accent's
 * primary action, or `danger` on a ruby row, because a status hue on a control
 * says the action *is* that state (see `Button`).
 *
 * The row classes are held in a map, which is the one place
 * `scripts/check-classes.mjs` cannot see (it reads `className` attributes
 * only). Every value here has been resolved against the compiled stylesheet by
 * hand; adding a tone means doing that again rather than assuming Tailwind
 * derives it.
 */
const ACTION_TONE: Record<ActionRowTone, string> = {
  ruby: 'border-rose-500/20 bg-rose-950/10',
  cyan: 'border-cyan-500/20 bg-cyan-950/10',
  amber: 'border-amber-500/20 bg-amber-950/10',
  emerald: 'border-emerald-500/20 bg-emerald-950/10',
};

const NEUTRAL_ROW = 'border-white/10 bg-white/[0.02]';

export interface ActionRowProps {
  icon: React.ReactNode;
  tone?: ActionRowTone;
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
      className={`flex items-center gap-3 rounded-xl border p-3.5 ${tone ? ACTION_TONE[tone] : NEUTRAL_ROW}`}
    >
      <div className={`shrink-0 ${tone ? TONE_TEXT[tone] : 'text-slate-400'}`}>{icon}</div>
      <div className="min-w-0 flex-1">
        <div className="text-xs font-bold text-slate-200">{title}</div>
        <div className="mt-0.5 text-[11px] leading-relaxed text-slate-400">{desc}</div>
      </div>
      <Button
        variant={tone === 'ruby' ? 'danger' : 'primary'}
        size="sm"
        onClick={onClick}
        disabled={disabled}
        title={disabled ? disabledReason : buttonTitle}
        className="shrink-0"
      >
        {buttonLabel}
      </Button>
    </div>
  );
}

export default ActionRow;
