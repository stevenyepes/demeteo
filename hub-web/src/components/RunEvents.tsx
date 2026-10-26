import { useId } from 'react';

import { elapsedText } from '../lib/fleetSummary';
import type { RunEvent } from '../lib/runs';

interface RunEventsProps {
  events: RunEvent[];
  /** Unix seconds, injected so the relative times are testable. */
  now: number;
}

/**
 * `kind` is shown as received, never mapped to a label: a kind this build has
 * not heard of is still the instance's word for what happened.
 */
export function RunEvents({ events, now }: RunEventsProps) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="nested-card px-4 py-3">
      <h2 id={headingId} className="font-heading text-lg font-semibold text-white">
        Events
      </h2>
      {events.length === 0 ? (
        <p className="mt-2">No events reported yet.</p>
      ) : (
        <ol aria-label="Events" className="mt-2 flex flex-col gap-1">
          {events.map((event) => (
            <li key={event.offset} className="flex flex-wrap items-baseline gap-x-4">
              <span className="font-mono text-xs text-slate-500">#{event.offset}</span>
              <span className="font-mono text-slate-200 break-all">{event.kind}</span>
              {event.stepId !== null && <span className="font-mono text-xs break-all">step {event.stepId}</span>}
              {event.ticketId !== null && (
                <span className="font-mono text-xs break-all">ticket {event.ticketId}</span>
              )}
              <time dateTime={new Date(event.occurredAt * 1000).toISOString()} className="ml-auto text-xs">
                {elapsedText(event.occurredAt, now)} ago
              </time>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
