import React from 'react';
import { Activity, ArrowRight, Check, CircleMinus, Lock, type LucideIcon } from 'lucide-react';

import { TONE_CHIP, type RunStatusTone } from '../../lib/runStatus';
import { ticketLabel } from '../../lib/discoveryProgress';
import { stateLabel, ticketNote, type TicketIndex } from '../../lib/ticketPresentation';
import { NODE_H, NODE_W } from '../../lib/ticketGraphLayout';
import type { TicketLane, TicketView } from '../../types';
import { Chip } from '../ui/Chip';

/**
 * Done-ness is a glyph, not a tint (`docs/PRD_DISCOVERY.md` §9.2): a check
 * once a prerequisite's PR merged, a lock while it has not. The tile carries
 * the verdict so an agent-built plan can be read for what is done first, and
 * the tint only agrees with it.
 */
const LANE_ICON: Record<TicketLane, LucideIcon> = {
  landed: Check,
  in_flight: Activity,
  ready: ArrowRight,
  blocked: Lock,
  dropped: CircleMinus,
};

const TONE_BORDER: Record<RunStatusTone, string> = {
  emerald: 'border-emerald-500/40',
  cyan: 'border-cyan-500/50 shadow-[0_0_18px_rgba(6,182,212,0.18)]',
  violet: 'border-violet-500/50 shadow-[0_0_18px_rgba(139,92,246,0.18)]',
  amber: 'border-amber-500/50 shadow-[0_0_18px_rgba(245,158,11,0.20)]',
  ruby: 'border-ruby-500/50',
  slate: 'border-slate-700/60',
};

const SELECTED =
  'border-cyan-400/70 shadow-[0_0_0_1px_rgba(34,211,238,0.4),0_0_18px_rgba(34,211,238,0.25)]';

interface TicketGraphNodeProps {
  view: TicketView;
  index: TicketIndex;
  tone: RunStatusTone;
  selected: boolean;
  x: number;
  y: number;
  onSelect: () => void;
}

export function TicketGraphNode({
  view,
  index,
  tone,
  selected,
  x,
  y,
  onSelect,
}: TicketGraphNodeProps): React.ReactElement {
  const lane = view.standing.lane;
  const Icon = LANE_ICON[lane];
  const dropped = lane === 'dropped';
  const note = ticketNote(view, index);

  return (
    <button
      type="button"
      data-testid="ticket-node"
      data-ticket={view.ticket.id}
      aria-pressed={selected}
      onClick={onSelect}
      // The one legitimate inline style here: a computed coordinate is the
      // datum, and no utility can express "wherever this plan's shape put it".
      style={{ left: x, top: y, width: NODE_W, height: NODE_H }}
      className={`absolute overflow-hidden rounded-xl border bg-slate-900/70 px-3.5 py-2.5 text-left shadow-[0_10px_15px_-3px_rgba(0,0,0,0.35)] backdrop-blur-[4px] transition-colors ${
        selected ? SELECTED : TONE_BORDER[tone]
      } ${dropped ? 'border-dashed opacity-50' : ''}`}
    >
      <div className="flex items-start gap-2.5">
        <span
          aria-hidden="true"
          className={`flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-lg border ${TONE_CHIP[tone]}`}
        >
          <Icon
            className={`h-[15px] w-[15px] ${lane === 'in_flight' ? 'animate-pulse motion-reduce:animate-none' : ''}`}
            strokeWidth={lane === 'landed' ? 2.5 : 2}
          />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block font-mono text-[9px] tracking-widest text-slate-500">
            {ticketLabel(view.ticket.seq)}
          </span>
          <span
            className={`line-clamp-2 block text-[13px] font-medium ${
              dropped
                ? 'text-slate-500 line-through'
                : lane === 'landed'
                  ? 'text-slate-300'
                  : 'text-slate-100'
            }`}
          >
            {view.ticket.title}
          </span>
        </span>
      </div>

      <div className="mt-2 flex flex-wrap items-center gap-1.5 pl-10">
        <Chip size="sm" tone={tone}>
          {stateLabel(view)}
        </Chip>
        {note && <span className="truncate font-mono text-[9px] text-slate-500">{note}</span>}
      </div>
    </button>
  );
}

export default TicketGraphNode;
