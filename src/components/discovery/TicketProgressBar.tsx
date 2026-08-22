import React from 'react';

interface TicketProgressBarProps {
  /** Both are percentages of the *live* ticket count, never of the total —
   *  `lib/discoveryProgress.ts` carries why. */
  landedPct: number;
  inFlightPct: number;
  /** Repeats the progress text, so the bar is readable without it. */
  title: string;
  className?: string;
}

/**
 * The two-segment bar Project Home's Discovery card and the workspace's
 * graph sub-header both draw. Emerald has landed, cyan is in flight, and the
 * unfilled remainder is everything still outstanding.
 *
 * Widths are inline because they are the datum, not a design token: there is
 * no utility for "however far this particular plan has got".
 */
export function TicketProgressBar({
  landedPct,
  inFlightPct,
  title,
  className = '',
}: TicketProgressBarProps): React.ReactElement {
  return (
    <div
      title={title}
      data-testid="ticket-progress-bar"
      className={`flex h-1 overflow-hidden rounded-full bg-white/[0.06] ${className}`}
    >
      <span
        data-testid="ticket-progress-landed"
        className="bg-emerald-500"
        style={{ width: `${landedPct}%` }}
      />
      <span
        data-testid="ticket-progress-in-flight"
        className="bg-cyan-500"
        style={{ width: `${inFlightPct}%` }}
      />
    </div>
  );
}

export default TicketProgressBar;
