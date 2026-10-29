import type { ReactNode } from 'react';

import type { RunStatusTone } from '../../../src/lib/runStatus';

/** The bar is a solid fill, so it takes the full-strength hue the chip maps do not carry. */
export const TONE_BAR: Record<RunStatusTone, string> = {
  emerald: 'bg-emerald-500',
  cyan: 'bg-cyan-500',
  amber: 'bg-amber-500',
  ruby: 'bg-ruby-500',
  slate: 'bg-slate-600',
};

interface ToneCardProps {
  tone: RunStatusTone;
  /** `panel` stands on the page; `nested` sits inside a panel (AGENTS.md §4). */
  surface: 'panel' | 'nested';
  /**
   * Edges the card in amber as well as barring it. Only for a card a human has
   * to act on: `tone` alone says what state the thing is in, this says the
   * card is the reason to look at the page.
   */
  needsYou?: boolean;
  className?: string;
  children: ReactNode;
}

/**
 * A card led by a status bar down its left edge: a run on an instance, a gate
 * waiting on the Fleet page. The bar is decoration — the card's own text names
 * the state, since hue alone carries none (AGENTS.md §4).
 */
export function ToneCard({ tone, surface, needsYou = false, className = '', children }: ToneCardProps) {
  return (
    <div
      className={`${surface === 'panel' ? 'panel' : 'nested-card'} relative overflow-hidden pl-5 ${
        needsYou ? 'border-amber-500/40' : ''
      } ${className}`}
    >
      <span aria-hidden="true" className={`absolute inset-y-0 left-0 w-1 ${TONE_BAR[tone]}`} />
      {children}
    </div>
  );
}
