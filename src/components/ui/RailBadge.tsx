import { TONE_BADGE, type RunStatusTone } from '../../lib/runStatus';

// The one count badge both rails render: the project rollup's amber/cyan pair
// (ProjectRailActivity) and the terminals entry's ruby/emerald pair
// (RailNavItem). Nothing in `checks.sh` fails on a duplicated Tailwind string,
// so a second copy of the geometry below would drift silently.

export interface RailBadgeProps {
  tone: RunStatusTone;
  count: number;
  /** Accessible name — the badge's own explanation, not its host's. */
  label: string;
  /** Pin to a corner of a collapsed rail button; omit for the expanded inline pill. */
  corner?: 'left' | 'right';
  pulse?: boolean;
  testId?: string;
}

const GEOMETRY = {
  left: 'absolute -top-1 -left-1 min-w-[15px] h-[15px] px-1',
  right: 'absolute -top-1 -right-1 min-w-[15px] h-[15px] px-1',
  inline: 'shrink-0 min-w-[18px] px-1.5 py-0.5',
} as const;

const SHAPE =
  'rounded-full flex items-center justify-center text-[10px] font-mono font-semibold border';

// A corner badge sits on a 36px button with its twin on the opposite corner;
// at `99+` each is ~26px wide and the pair overlap across the button face.
const CAP = { corner: 9, inline: 99 } as const;

export function RailBadge({
  tone,
  count,
  label,
  corner,
  pulse = false,
  testId,
}: RailBadgeProps): React.ReactElement {
  const glow = tone === 'amber' ? 'animate-pulse-glow-amber' : 'animate-pulse-glow';
  const cap = corner ? CAP.corner : CAP.inline;
  const className = [GEOMETRY[corner ?? 'inline'], SHAPE, TONE_BADGE[tone], pulse && glow]
    .filter(Boolean)
    .join(' ');

  return (
    // `role="img"`, not a bare `<span>`: a span maps to the ARIA `generic`
    // role, which *prohibits* an author-supplied name, so a conformant screen
    // reader drops the `aria-label` entirely. `toHaveAccessibleName` cannot
    // catch that regression — `dom-accessibility-api` does not model
    // prohibition, so it reports the name a real user never hears.
    <span role="img" data-testid={testId} aria-label={label} title={label} className={className}>
      {count > cap ? `${cap}+` : count}
    </span>
  );
}

export default RailBadge;
