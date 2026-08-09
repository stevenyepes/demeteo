/**
 * StatusBadge — the standalone glow dot (UI redesign plan §5.1).
 *
 * `Chip` owns every labelled pill; this badge kept the one job a pill cannot
 * do — a label-less liveness mark in a rail or list row, where the row's own
 * text is the label. It is not a smaller `Chip`: `Chip`'s dot is `bg-current`
 * and only exists inside a pill, so neither substitutes for the other.
 */

import { runStatusMeta, type RunStatusTone } from '../../lib/runStatus';

/** Statuses StatusBadge renders that aren't run statuses (machine /
 *  step vocabulary), mapped straight to a tone. Everything else defers
 *  to the shared run-status vocabulary (`lib/runStatus.ts`, F27) so the
 *  badge can't drift from the rest of the app. */
const NON_RUN_TONES: Record<string, RunStatusTone> = {
  idle: 'emerald',
  active: 'emerald',
  skipped: 'slate',
};

/** Dot rendering per tone: solid fill + a soft glow for anything that
 *  isn't inert. Component-specific (the glow shadow only exists here),
 *  so it lives beside the badge rather than in the shared tone maps. */
const TONE_DOT: Record<RunStatusTone, string> = {
  emerald: 'bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.8)]',
  cyan: 'bg-cyan-500 shadow-[0_0_8px_rgba(6,182,212,0.8)]',
  violet: 'bg-violet-500 shadow-[0_0_8px_rgba(139,92,246,0.8)]',
  amber: 'bg-amber-500 shadow-[0_0_8px_rgba(245,158,11,0.8)]',
  ruby: 'bg-ruby-500 shadow-[0_0_8px_rgba(239,68,68,0.8)]',
  slate: 'bg-slate-500',
};

interface StatusBadgeProps {
  status: string;
  className?: string;
}

export function StatusBadge({ status, className = '' }: StatusBadgeProps) {
  const normalized = status.toLowerCase();
  const tone = NON_RUN_TONES[normalized] ?? runStatusMeta(normalized).tone;

  return (
    <div
      data-testid="status-badge"
      data-tone={tone}
      className={`w-2 h-2 rounded-full shrink-0 ${TONE_DOT[tone]} ${className}`}
    />
  );
}
