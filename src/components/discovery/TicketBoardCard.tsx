import React from 'react';

import { ticketLabel } from '../../lib/discoveryProgress';
import type { RunStatusTone } from '../../lib/runStatus';
import { ticketNote, type TicketIndex } from '../../lib/ticketPresentation';
import type { TicketView } from '../../types';
import { Chip } from '../ui/Chip';

const TONE_BORDER: Record<RunStatusTone, string> = {
  emerald: 'border-emerald-500/40',
  cyan: 'border-cyan-500/50',
  violet: 'border-violet-500/50',
  amber: 'border-amber-500/50',
  ruby: 'border-ruby-500/50',
  slate: 'border-slate-700/60',
};

const LANE_DOT: Record<RunStatusTone, string> = {
  emerald: 'bg-emerald-400',
  cyan: 'bg-cyan-400',
  violet: 'bg-violet-400',
  amber: 'bg-amber-400',
  ruby: 'bg-ruby-400',
  slate: 'bg-slate-500',
};

interface TicketBoardCardProps {
  view: TicketView;
  index: TicketIndex;
  tone: RunStatusTone;
  selected: boolean;
  onSelect: () => void;
}

/**
 * One ticket in a lane. **Click to select is the only interaction** — §9.2 and
 * §6.6 both refuse a drag here: a lane is derived from the edges on every
 * render, so there is no column to drop into, and the three things a drag
 * could plausibly mean (edit an edge, drop the ticket, force start it) are all
 * explicit acts with their own record.
 */
export function TicketBoardCard({
  view,
  index,
  tone,
  selected,
  onSelect,
}: TicketBoardCardProps): React.ReactElement {
  const lane = view.standing.lane;
  const dropped = lane === 'dropped';
  const note = ticketNote(view, index);

  return (
    <button
      type="button"
      data-testid="ticket-card"
      data-ticket={view.ticket.id}
      aria-pressed={selected}
      onClick={onSelect}
      className={`w-[190px] rounded-[10px] border bg-slate-900/70 px-3 py-2.5 text-left transition hover:-translate-y-px ${
        selected
          ? 'border-cyan-400/70 shadow-[0_0_0_1px_rgba(34,211,238,0.4),0_0_18px_rgba(34,211,238,0.25)]'
          : TONE_BORDER[tone]
      } ${dropped ? 'border-dashed opacity-50' : ''}`}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono text-[9px] tracking-widest text-slate-500">
          {ticketLabel(view.ticket.seq)}
        </span>
        {view.ticket.agent_kind && (
          <Chip size="sm" tone={tone} maxWidth="7rem">
            {view.ticket.agent_kind}
          </Chip>
        )}
      </div>

      <p
        className={`m-0 mt-1 text-xs font-medium leading-snug ${
          dropped
            ? 'text-slate-500 line-through decoration-slate-500/50'
            : lane === 'landed'
              ? 'text-slate-400'
              : 'text-slate-100'
        }`}
      >
        {view.ticket.title}
      </p>

      {note && (
        <div className="mt-2 flex items-center gap-1.5">
          <span aria-hidden="true" className={`h-1.5 w-1.5 shrink-0 rounded-full ${LANE_DOT[tone]}`} />
          <span className="truncate font-mono text-[9px] text-slate-500">{note}</span>
        </div>
      )}
    </button>
  );
}

export default TicketBoardCard;
