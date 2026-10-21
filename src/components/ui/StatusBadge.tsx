/**
 * StatusBadge — the standalone status dot (UI redesign plan §5.1).
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

/** A solid fill per tone. The shared maps carry tints and inks, and a dot
 *  needs the full-strength hue, so this one lives beside the badge. */
const TONE_DOT: Record<RunStatusTone, string> = {
  emerald: 'bg-emerald-500',
  cyan: 'bg-cyan-500',
  amber: 'bg-amber-500',
  ruby: 'bg-ruby-500',
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
