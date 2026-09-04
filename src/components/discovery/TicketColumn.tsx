import React, { useState } from 'react';
import { Compass, Kanban, Workflow } from 'lucide-react';

import { progressSegments, progressText } from '../../lib/discoveryProgress';
import type { TicketIndex } from '../../lib/ticketPresentation';
import type { TicketProgress, TicketView } from '../../types';
import EmptyStateCard from '../EmptyStateCard';
import { SegmentedControl } from '../ui/SegmentedControl';
import { ColumnSubHeader } from './ColumnSubHeader';
import { TicketBoard } from './TicketBoard';
import { TicketGraph } from './TicketGraph';
import { TicketProgressBar } from './TicketProgressBar';

type TicketViewMode = 'graph' | 'board';

const VIEW_OPTIONS = [
  { value: 'graph' as const, label: 'Graph', icon: Workflow },
  { value: 'board' as const, label: 'Board', icon: Kanban },
];

interface TicketColumnProps {
  tickets: TicketView[];
  index: TicketIndex;
  /** `null` until `discovery_board` answers. */
  progress: TicketProgress | null;
  selectedId: string | null;
  onSelect: (ticketId: string) => void;
  /** Hides via class + `aria-hidden`, not unmounting — the graph's zoom state
   *  must survive a `'stacked'`-mode toggle. */
  hidden?: boolean;
}

/**
 * The two views over one ticket set (`DISCOVERY_UI_SPEC.md` §3.5, PRD §9.2).
 *
 * **Both read the same `discovery_board` call**, which is the whole reason the
 * second view is safe to have: a ticket cannot be done on the board and
 * blocked in the graph when there is one answer between them, and nothing here
 * recomputes a lane the backend already derived.
 *
 * The toggle is `SegmentedControl` unchanged, and so is its cyan selection.
 * `docs/TASKS_DISCOVERY.md` settles that against the mock's violet: a view
 * toggle takes no action, and violet is the primary-action colour.
 */
export function TicketColumn({
  tickets,
  index,
  progress,
  selectedId,
  onSelect,
  hidden = false,
}: TicketColumnProps): React.ReactElement {
  const [mode, setMode] = useState<TicketViewMode>('graph');

  const text = progress ? progressText(progress) : null;
  const segments = progress ? progressSegments(progress) : null;

  return (
    <div
      className={`flex min-w-0 min-h-0 flex-1 flex-col ${hidden ? 'hidden' : ''}`}
      aria-hidden={hidden ? 'true' : undefined}
    >
      <ColumnSubHeader
        left={
          <SegmentedControl
            options={VIEW_OPTIONS}
            value={mode}
            onChange={setMode}
            size="sm"
            ariaLabel="Ticket view"
          />
        }
      >
        {text && segments && (
          <>
            <span className="font-mono text-[10px] text-slate-400">{text}</span>
            <TicketProgressBar
              landedPct={segments.landedPct}
              inFlightPct={segments.inFlightPct}
              title={text}
              className="w-24 shrink-0"
            />
          </>
        )}
      </ColumnSubHeader>

      <div className="relative min-h-0 flex-1">
        {tickets.length === 0 ? (
          <div className="absolute inset-0 overflow-y-auto bg-[#050608] p-6">
            <EmptyStateCard
              variant="inline"
              icon={Compass}
              title="No tickets yet"
              description="This discovery has proposed nothing so far. Decompose the interview when the shape is settled, and the tickets it produces appear here with their edges."
            />
          </div>
        ) : mode === 'graph' ? (
          <TicketGraph
            tickets={tickets}
            index={index}
            selectedId={selectedId}
            onSelect={onSelect}
          />
        ) : (
          <TicketBoard
            tickets={tickets}
            index={index}
            selectedId={selectedId}
            onSelect={onSelect}
          />
        )}
      </div>
    </div>
  );
}

export default TicketColumn;
