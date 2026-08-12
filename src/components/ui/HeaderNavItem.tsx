import type { LucideIcon } from 'lucide-react';

import type { HeaderDensity } from '../../lib/headerLayout';

// One nav entry in the top header bar, rendered at the density headerLayout.ts
// decides (UI_REDESIGN_PLAN §5.1).
//
// **Why RailNavItem could not take this job.** Both of its variants are sized
// for a vertical rail — expanded is `w-full`, collapsed is a `w-10 h-10` square
// — and neither is an intrinsic-width item in a horizontal row. Its badge
// vocabulary is emerald-count plus ruby-attention, where the header's is
// amber-actionable plus cyan-activity plus emerald-pulse. Folding the two
// together would give one file three layout modes and two badge vocabularies.
//
// **At `icons` density the label text node is not rendered**, rather than
// hidden with CSS: a CSS-hidden label stays in the accessibility tree and in
// `queryByText`, so it would misreport the control to a screen reader while
// still passing for absent to the eye. The accessible name survives both
// densities through `aria-label`.
//
// `aria-label` *overrides* an element's contents in the accessible-name
// computation, so the count badge below is unreachable to a screen reader on
// its own no matter how it is marked up — the name has to carry the count, and
// `title` cannot stand in because it loses to a published name too.

export type HeaderNavAccent = 'violet' | 'cyan' | 'amber';

const ACCENT_CLASS: Record<HeaderNavAccent, string> = {
  violet: 'text-violet-400',
  cyan: 'text-cyan-400',
  amber: 'text-amber-400',
};

export interface HeaderNavItemProps {
  icon: LucideIcon;
  label: string;
  density: HeaderDensity;
  /** Icon tint, per the header's accent mapping — violet is primary, cyan is
   *  interactive, amber is actionable. Omitted, the icon inherits the button's
   *  own text colour. */
  accent?: HeaderNavAccent;
  active?: boolean;
  /** Amber actionable count — rendered as a corner badge when > 0. */
  count?: number;
  /** Cyan "work in progress" dot — rendered only when `count` is absent/0. */
  activity?: boolean;
  /** Emerald `animate-pulse-glow` dot — "changing on its own". */
  pulse?: boolean;
  title?: string;
  /** Accessible name, where it has to differ from the visible label. The
   *  header's Terminals entry publishes `Open terminals view`, a name that
   *  predates the label and is pinned by a test elsewhere; every other entry
   *  leaves this unset and is named by its label. */
  ariaLabel?: string;
  /** Test id for the button; the count badge takes `${testId}-badge`. */
  testId?: string;
  /** Test id for the activity/pulse dot, which share one corner slot. */
  pulseTestId?: string;
  onClick: () => void;
}

function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(' ');
}

export function HeaderNavItem({
  icon: Icon,
  label,
  density,
  accent,
  active = false,
  count,
  activity = false,
  pulse = false,
  title,
  ariaLabel,
  testId,
  pulseTestId,
  onClick,
}: HeaderNavItemProps): React.ReactElement {
  const badgeText =
    typeof count !== 'number' || count <= 0 ? null : count > 9 ? '9+' : String(count);
  const hasCount = badgeText !== null;
  const showActivity = !hasCount && activity;
  const showPulse = !hasCount && !activity && pulse;
  const name = ariaLabel ?? label;

  return (
    <button
      type="button"
      onClick={onClick}
      title={title ?? label}
      aria-label={hasCount ? `${name} ${badgeText}` : name}
      data-testid={testId}
      data-active={active ? 'true' : 'false'}
      aria-current={active ? 'page' : undefined}
      className={cx(
        'relative shrink-0 flex items-center gap-1.5 px-2 py-1.5 rounded text-xs transition-colors',
        active
          ? 'bg-white/[0.04] text-white'
          : 'text-slate-400 hover:bg-white/5 hover:text-white',
      )}
    >
      <Icon className={cx('w-4 h-4 shrink-0', accent && ACCENT_CLASS[accent])} />
      {density === 'labels' && <span className="font-mono whitespace-nowrap">{label}</span>}
      {hasCount ? (
        <span
          data-testid={testId ? `${testId}-badge` : undefined}
          data-badge="count"
          className="absolute -top-1 -right-1 min-w-[16px] h-4 px-1 rounded-full bg-amber-500 text-slate-950 text-[10px] font-bold leading-4 text-center"
        >
          {badgeText}
        </span>
      ) : showActivity ? (
        <span
          data-testid={pulseTestId}
          data-badge="activity"
          className="absolute -top-0.5 -right-0.5 w-2 h-2 rounded-full bg-cyan-400 animate-pulse"
        />
      ) : showPulse ? (
        <span
          data-testid={pulseTestId}
          data-badge="pulse"
          className="absolute -top-0.5 -right-0.5 w-2 h-2 rounded-full bg-emerald-400 animate-pulse-glow"
        />
      ) : null}
    </button>
  );
}

export default HeaderNavItem;
