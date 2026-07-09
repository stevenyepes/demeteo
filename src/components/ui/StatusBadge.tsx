import { runStatusMeta, TONE_CHIP, type RunStatusTone } from '../../lib/runStatus';

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
  variant?: 'dot' | 'pill';
  label?: string;
  className?: string;
}

export function StatusBadge({ status, variant = 'dot', label, className = '' }: StatusBadgeProps) {
  const normalized = status.toLowerCase();
  const tone = NON_RUN_TONES[normalized] ?? runStatusMeta(normalized).tone;

  if (variant === 'dot') {
    return (
      <div className={`w-2 h-2 rounded-full shrink-0 ${TONE_DOT[tone]} ${className}`} />
    );
  }

  const display = label ?? status.replace(/_/g, ' ');
  return (
    <span
      className={`inline-flex items-center px-2 py-0.5 rounded text-xs font-medium border capitalize ${TONE_CHIP[tone]} ${className}`}
    >
      {display}
    </span>
  );
}
