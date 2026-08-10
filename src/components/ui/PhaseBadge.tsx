/**
 * PhaseBadge — maps a terminal session phase to a small labelled chip with an
 * icon + colour (spec §5). Colours, icons, and Tailwind style mirror the
 * inline phase ternary in `src/components/TerminalSurface.tsx` so the two
 * stay in sync.
 */

import { AlertCircle, RotateCw, Wifi, WifiOff } from 'lucide-react';

export type TerminalPhase = 'connecting' | 'running' | 'disconnected' | 'closed' | 'error';

export interface PhaseBadgeProps {
  phase: TerminalPhase;
  className?: string;
}

interface PhaseMeta {
  Icon: typeof AlertCircle;
  /** Extra icon class (e.g. an animation) appended to the base `w-3 h-3`. */
  iconClassName?: string;
  label: string;
  color: string;
}

const PHASE_META: Record<TerminalPhase, PhaseMeta> = {
  connecting: { Icon: RotateCw, iconClassName: 'animate-spin', label: 'Connecting', color: 'text-amber-400' },
  running: { Icon: Wifi, label: 'Running', color: 'text-emerald-400' },
  disconnected: { Icon: WifiOff, label: 'Disconnected', color: 'text-amber-400' },
  closed: { Icon: WifiOff, label: 'Closed', color: 'text-slate-500' },
  error: { Icon: AlertCircle, label: 'Error', color: 'text-ruby-400' },
};

export function PhaseBadge({ phase, className = '' }: PhaseBadgeProps): React.ReactElement {
  // `phase` originates across the IPC boundary (`TerminalTabDescriptor`),
  // so fall back defensively rather than destructuring `undefined` and
  // crashing the whole row if an unknown value ever arrives.
  const { Icon, iconClassName, label, color } = PHASE_META[phase] ?? PHASE_META.closed;

  return (
    <span
      data-testid="phase-badge"
      data-phase={phase}
      className={`inline-flex items-center gap-1.5 text-[10px] font-mono ${color} ${className}`}
    >
      <Icon className={`w-3 h-3${iconClassName ? ` ${iconClassName}` : ''}`} />
      <span>{label}</span>
    </span>
  );
}
