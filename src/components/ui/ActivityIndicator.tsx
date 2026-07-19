import React from 'react';
import { Loader2 } from 'lucide-react';

import type { TerminalActivity } from '../../types';

export interface ActivityIndicatorProps {
  /** Live activity of the agent in the tab. Renders nothing when null —
   *  a plain shell (or an agent we can't read yet) carries no mark. */
  activity: TerminalActivity;
  className?: string;
}

/** Human-readable label per state — surfaced via `title` + `aria-label` so
 *  the mark is legible to a hover and to a screen reader (spec
 *  `TERMINAL_ACTIVITY_UX` §2). */
const ACTIVITY_LABEL: Record<'working' | 'awaiting_input' | 'awaiting_approval', string> = {
  working: 'Working',
  awaiting_input: 'Waiting for you',
  awaiting_approval: 'Needs a decision',
};

/**
 * The at-a-glance activity mark that sits beside the `AgentBadge` (spec
 * `TERMINAL_ACTIVITY_UX` §2/§3). Pure presentational — keyed only on
 * `activity`, it renders nothing for `null`. The visual weight is
 * deliberately ordered so *needs a decision* out-shouts *waiting*, which
 * out-shouts *working*:
 *
 *   working          → violet animated spinner (matches AgentBadge's violet)
 *   awaiting_input   → steady amber dot ("your turn")
 *   awaiting_approval→ pulsing red-amber dot, highest salience
 */
function ActivityIndicatorImpl({
  activity,
  className = '',
}: ActivityIndicatorProps): React.ReactElement | null {
  if (!activity) return null;

  const label = ACTIVITY_LABEL[activity];

  if (activity === 'working') {
    return (
      <span
        data-testid="activity-indicator"
        data-activity={activity}
        role="status"
        title={label}
        aria-label={label}
        className={['inline-flex items-center text-violet-300 shrink-0', className].join(' ')}
      >
        <Loader2 className="w-2.5 h-2.5 shrink-0 animate-spin" aria-hidden="true" />
      </span>
    );
  }

  // awaiting_input / awaiting_approval both render a dot; approval pulses in
  // a hotter red-amber and steady amber marks a plain "your turn".
  const dotClasses =
    activity === 'awaiting_approval'
      ? 'bg-ruby-400 animate-pulse-glow'
      : 'bg-amber-400';

  return (
    <span
      data-testid="activity-indicator"
      data-activity={activity}
      role="status"
      title={label}
      aria-label={label}
      className={['inline-flex items-center shrink-0', className].join(' ')}
    >
      <span className={['w-2 h-2 rounded-full shrink-0', dotClasses].join(' ')} aria-hidden="true" />
    </span>
  );
}

export const ActivityIndicator = React.memo(ActivityIndicatorImpl);

export default ActivityIndicator;
