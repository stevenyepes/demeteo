import type { LucideIcon } from 'lucide-react';

// A single button in the left project rail (TERMINALS_VIEW_SPEC §5, §6) — used
// for the "Terminals" entry. Two variants share one component so the rail can
// render the same item at both widths: expanded (`w-60`, icon + label + optional
// count badge + optional pulse) and collapsed (`w-14`, icon-only with a corner
// count badge and the pulse dot). Active styling matches the rest of the rail
// buttons in ProjectRail.tsx.

export interface RailNavItemProps {
  icon: LucideIcon;
  label: string;
  active?: boolean;
  /** When > 0, render a small count badge. */
  count?: number;
  /** When > 0, render a high-salience attention badge — terminals that
   *  need a decision (spec `TERMINAL_ACTIVITY` §3). Ships wired at 0 in
   *  Phase 1 and lights up once the backend emits `awaiting_approval`. */
  attentionCount?: number;
  /** When true, show a small emerald pulse dot (e.g. live sessions). */
  pulse?: boolean;
  /** Collapsed rail (icon-only) vs expanded. */
  collapsed?: boolean;
  onClick: () => void;
  className?: string;
}

function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(' ');
}

export function RailNavItem({
  icon: Icon,
  label,
  active = false,
  count,
  attentionCount,
  pulse = false,
  collapsed = false,
  onClick,
  className = '',
}: RailNavItemProps): React.ReactElement {
  const hasCount = typeof count === 'number' && count > 0;
  const hasAttention = typeof attentionCount === 'number' && attentionCount > 0;

  const activeClasses = active
    ? 'bg-white/[0.07] border border-white/10 text-white'
    : 'border border-transparent text-slate-400 hover:bg-white/5 hover:text-slate-200';

  const common = {
    type: 'button' as const,
    onClick,
    title: label,
    'aria-label': label,
    'data-testid': 'rail-nav-item',
    'data-active': active ? 'true' : 'false',
    'aria-current': active ? ('page' as const) : undefined,
  };

  if (collapsed) {
    return (
      <button
        {...common}
        className={cx(
          'relative w-10 h-10 rounded-lg flex items-center justify-center bg-[#0d0f14] transition-colors',
          activeClasses,
          className,
        )}
      >
        <Icon className="w-4 h-4" />
        {hasCount && (
          <span className="absolute -top-1 -right-1 min-w-[15px] h-[15px] px-1 rounded-full flex items-center justify-center text-[10px] font-mono font-semibold bg-emerald-500/20 text-emerald-300 border border-emerald-500/30">
            {count}
          </span>
        )}
        {hasAttention && (
          <span
            data-testid="rail-nav-attention"
            aria-label={`${attentionCount} awaiting your decision`}
            className="absolute -top-1 -left-1 min-w-[15px] h-[15px] px-1 rounded-full flex items-center justify-center text-[10px] font-mono font-semibold bg-ruby-500/20 text-ruby-300 border border-ruby-500/40 animate-pulse-glow"
          >
            {attentionCount}
          </span>
        )}
        {pulse && (
          <span className="absolute bottom-1 right-1 w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse-glow" />
        )}
      </button>
    );
  }

  return (
    <button
      {...common}
      className={cx(
        'w-full flex items-center gap-2.5 px-3 py-2 rounded-lg bg-[#0d0f14] transition-colors',
        activeClasses,
        className,
      )}
    >
      <Icon className="w-4 h-4 shrink-0" />
      <span className="text-xs font-medium truncate flex-1 text-left">{label}</span>
      {hasAttention && (
        <span
          data-testid="rail-nav-attention"
          aria-label={`${attentionCount} awaiting your decision`}
          className="shrink-0 min-w-[18px] px-1.5 py-0.5 rounded-full flex items-center justify-center text-[10px] font-mono font-semibold bg-ruby-500/20 text-ruby-300 border border-ruby-500/40 animate-pulse-glow"
        >
          {attentionCount}
        </span>
      )}
      {pulse && (
        <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse-glow shrink-0" />
      )}
      {hasCount && (
        <span className="shrink-0 min-w-[18px] px-1.5 py-0.5 rounded-full flex items-center justify-center text-[10px] font-mono font-semibold bg-emerald-500/20 text-emerald-300 border border-emerald-500/30">
          {count}
        </span>
      )}
    </button>
  );
}

export default RailNavItem;
