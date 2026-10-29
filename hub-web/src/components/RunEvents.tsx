import { useId, useState } from 'react';

import { buttonClasses } from '../../../src/components/ui/Button';
import { elapsedText } from '../lib/fleetSummary';
import type { RunEvent } from '../lib/runs';

interface RunEventsProps {
  events: RunEvent[];
  /** Unix seconds, injected so the relative times are testable. */
  now: number;
  /** A step's title by its id; a step not in it is named by its id. */
  stepTitles?: Readonly<Record<string, string>>;
}

/**
 * The twelve kinds the protocol carries (`RunEventKind`). A kind missing here
 * is shown as received rather than dropped: it is still the instance's word
 * for what happened, sent by a newer build than this one.
 */
const KIND_LABELS: Readonly<Record<string, string>> = {
  feature_started: 'Run started',
  feature_completed: 'Run completed',
  feature_failed: 'Run failed',
  feature_cancelled: 'Run cancelled',
  step_started: 'Step started',
  step_completed: 'Step completed',
  step_failed: 'Step failed',
  ticket_started: 'Ticket started',
  ticket_completed: 'Ticket completed',
  ticket_failed: 'Ticket failed',
  gate_pending: 'Gate waiting',
  gate_decided: 'Gate decided',
};

/**
 * A gate waiting for hours retries its step on every wake, so a long run
 * reports hundreds of events; the latest few are what the page is read for.
 */
const SHOWN = 8;

export function RunEvents({ events, now, stepTitles = {} }: RunEventsProps) {
  const headingId = useId();
  const [showAll, setShowAll] = useState(false);
  const newestFirst = [...events].sort((a, b) => b.offset - a.offset);
  const shown = showAll ? newestFirst : newestFirst.slice(0, SHOWN);
  return (
    <section aria-labelledby={headingId} className="panel p-5 text-sm text-slate-400">
      <h2 id={headingId} className="font-heading text-base font-semibold text-white">
        Activity
      </h2>
      {events.length === 0 ? (
        <p className="mt-3">No events reported yet.</p>
      ) : (
        <>
          <ol aria-label="Events" className="mt-3 flex flex-col">
            {shown.map((event) => (
              <li
                key={event.offset}
                className="grid grid-cols-[4rem_minmax(0,1fr)_auto] items-baseline gap-3 border-t border-hairline py-2"
              >
                <span className="font-mono text-xs text-slate-500">#{event.offset}</span>
                <span className="break-words text-slate-200">
                  {KIND_LABELS[event.kind] ?? <span className="font-mono text-xs">{event.kind}</span>}
                  {event.stepId !== null && (
                    <span className="text-slate-400"> · {stepTitles[event.stepId] ?? event.stepId}</span>
                  )}
                  {event.ticketId !== null && (
                    <span className="font-mono text-xs text-slate-400"> · {event.ticketId}</span>
                  )}
                </span>
                <time
                  dateTime={new Date(event.occurredAt * 1000).toISOString()}
                  className="font-mono text-xs text-slate-500"
                >
                  {elapsedText(event.occurredAt, now)} ago
                </time>
              </li>
            ))}
          </ol>
          {events.length > SHOWN && (
            <button
              type="button"
              onClick={() => setShowAll((all) => !all)}
              className={`${buttonClasses('ghost', 'sm')} mt-2`}
            >
              {showAll ? 'Show the latest only' : `Show all ${events.length} events`}
            </button>
          )}
        </>
      )}
    </section>
  );
}
