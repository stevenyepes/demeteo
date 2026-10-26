import { useId } from 'react';

import { elapsedText } from '../lib/fleetSummary';
import type { RunDetail } from '../lib/runs';
import { ticketRows } from '../lib/runView';

interface TicketProgressProps {
  detail: Pick<RunDetail, 'tickets' | 'events'>;
  /** Unix seconds, injected so the relative times are testable. */
  now: number;
}

/** Ids, kinds and counts only: the mirror carries no ticket body, log or diff to open. */
export function TicketProgress({ detail, now }: TicketProgressProps) {
  const headingId = useId();
  const rows = ticketRows(detail.events);
  const { completed, failed, running } = detail.tickets;
  return (
    <section aria-labelledby={headingId} className="nested-card px-4 py-3">
      <h2 id={headingId} className="font-heading text-lg font-semibold text-white">
        Tickets
      </h2>
      <dl aria-label="Ticket counts" className="mt-2 flex flex-wrap gap-x-6 gap-y-1">
        <div className="flex items-baseline gap-2">
          <dt>Completed</dt>
          <dd className="text-slate-200">{completed}</dd>
        </div>
        <div className="flex items-baseline gap-2">
          <dt>Running</dt>
          <dd className="text-emerald-300">{running}</dd>
        </div>
        <div className="flex items-baseline gap-2">
          <dt>Failed</dt>
          <dd className="text-ruby-300">{failed}</dd>
        </div>
      </dl>
      {rows.length === 0 ? (
        <p className="mt-2">No ticket events reported yet.</p>
      ) : (
        <ul aria-label="Tickets" className="mt-2 flex flex-col gap-1">
          {rows.map((row) => (
            <li key={row.ticketId} className="flex flex-wrap items-baseline gap-x-4">
              <span className="font-mono text-slate-200 break-all">{row.ticketId}</span>
              <span className="font-mono text-xs break-all">{row.lastKind}</span>
              <span className="ml-auto text-xs">{elapsedText(row.lastAt, now)} ago</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
