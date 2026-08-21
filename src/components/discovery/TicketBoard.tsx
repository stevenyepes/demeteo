import React from 'react';

import { TONE_TEXT } from '../../lib/runStatus';
import { bucketByLane, ticketTone, type TicketIndex } from '../../lib/ticketPresentation';
import type { TicketView } from '../../types';
import { TicketBoardCard } from './TicketBoardCard';

const LANE_DOT = {
  blocked: 'bg-amber-400',
  ready: 'bg-violet-400',
  in_flight: 'bg-cyan-400',
  landed: 'bg-emerald-400',
  dropped: 'bg-slate-500',
} as const;

interface TicketBoardProps {
  tickets: TicketView[];
  index: TicketIndex;
  selectedId: string | null;
  onSelect: (ticketId: string) => void;
}

/**
 * How much is done — the same tickets the graph draws, in the five lanes
 * §9.2 names.
 *
 * All five render whatever they hold, empty included: a lane that disappears
 * when it empties makes the board's shape depend on the plan's progress, and
 * the point of the second view is that its shape does not.
 */
export function TicketBoard({
  tickets,
  index,
  selectedId,
  onSelect,
}: TicketBoardProps): React.ReactElement {
  return (
    <div
      data-testid="ticket-board"
      className="absolute inset-0 flex flex-col gap-[18px] overflow-y-auto bg-[#050608] px-3 py-4"
    >
      {bucketByLane(tickets).map(({ meta, tickets: inLane }) => (
        <section key={meta.lane} data-testid={`lane-${meta.lane}`}>
          <div className="flex items-center gap-2">
            <span aria-hidden="true" className={`h-1.5 w-1.5 rounded-full ${LANE_DOT[meta.lane]}`} />
            <h3
              className={`m-0 font-heading text-[11px] font-semibold tracking-wide ${TONE_TEXT[meta.tone]}`}
            >
              {meta.label}
            </h3>
            <span className="font-mono text-[10px] text-slate-500">{inLane.length}</span>
            <span aria-hidden="true" className="h-px flex-1 bg-white/5" />
            <span className="font-mono text-[10px] text-slate-500">{meta.note}</span>
          </div>

          <div className="mt-2.5 flex flex-wrap gap-2.5">
            {inLane.length === 0 ? (
              <p className="m-0 text-[11px] text-slate-600">Nothing here.</p>
            ) : (
              inLane.map((view) => (
                <TicketBoardCard
                  key={view.ticket.id}
                  view={view}
                  index={index}
                  tone={ticketTone(view, index)}
                  selected={selectedId === view.ticket.id}
                  onSelect={() => onSelect(view.ticket.id)}
                />
              ))
            )}
          </div>
        </section>
      ))}
    </div>
  );
}

export default TicketBoard;
