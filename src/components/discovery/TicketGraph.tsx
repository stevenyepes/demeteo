import React, { useMemo } from 'react';

import { layoutTicketGraph } from '../../lib/ticketGraphLayout';
import { ticketTone, type TicketIndex } from '../../lib/ticketPresentation';
import type { TicketView } from '../../types';
import { useCanvasViewport } from '../../hooks/useCanvasViewport';
import { CanvasZoomControls } from '../ui/CanvasZoomControls';
import { TicketGraphNode } from './TicketGraphNode';

interface TicketGraphProps {
  tickets: TicketView[];
  index: TicketIndex;
  selectedId: string | null;
  onSelect: (ticketId: string) => void;
}

/** The five buckets, in the order §3.5.3 lists them. */
const LEGEND: readonly { label: string; dot: string }[] = [
  { label: 'Blocked', dot: 'bg-amber-400' },
  { label: 'Ready', dot: 'bg-violet-400' },
  { label: 'In flight', dot: 'bg-cyan-400' },
  { label: 'Landed', dot: 'bg-emerald-400' },
  { label: 'Dropped', dot: 'bg-slate-500' },
];

/**
 * What depends on what.
 *
 * Pan and zoom are `useCanvasViewport`; the fit it frames with preserves
 * aspect, so one axis is left over whenever the pane's is not the graph's —
 * and the pane's changes on its own now that the interview column can be
 * hidden. The leftover is centred by `m-auto` on a flex item rather than
 * `justify-center` on the scroller: with content larger than the viewport,
 * centring the *container* puts the overflow's leading edge outside the scroll
 * range and nothing can reach it, while auto margins collapse to zero once
 * there is no free space. `shrink-0` is the third of the three — the canvas has
 * a definite width, and a flex item would otherwise shrink to the pane and take
 * the horizontal scroll with it.
 *
 * **Not `WorkflowCanvas`** — `docs/TASKS_DISCOVERY.md` records why: reuse would
 * import the run-tone vocabulary the canvas exists to paint, and a ticket lane
 * is not a run status. React Flow and its layout worker would still be cost
 * with no payer; the wheel and drag this pane answers are a hook and a scroller.
 *
 * Selection lights the *incident* edges, derived from the selected node rather
 * than from the hand-listed pairs the mock's stylesheet enumerates.
 */
export function TicketGraph({
  tickets,
  index,
  selectedId,
  onSelect,
}: TicketGraphProps): React.ReactElement {
  const layout = useMemo(() => layoutTicketGraph(tickets), [tickets]);
  const viewport = useCanvasViewport({ width: layout.width, height: layout.height });

  return (
    <div data-testid="ticket-graph" className="absolute inset-0">
      <div
        ref={viewport.paneRef}
        {...viewport.panProps}
        className={`absolute inset-0 flex touch-none overflow-auto bg-[#050608] bg-[radial-gradient(#334155_1px,transparent_1px)] bg-[length:20px_20px] ${
          viewport.panning ? 'cursor-grabbing select-none' : 'cursor-grab'
        }`}
      >
        <div
          ref={viewport.canvasRef}
          data-testid="ticket-graph-canvas"
          className="m-auto shrink-0"
          style={{
            width: layout.width * viewport.zoom,
            height: layout.height * viewport.zoom,
          }}
        >
          <div
            className="relative origin-top-left"
            style={{
              width: layout.width,
              height: layout.height,
              transform: `scale(${viewport.zoom})`,
            }}
          >
            <svg
              aria-hidden="true"
              width={layout.width}
              height={layout.height}
              className="pointer-events-none absolute inset-0 overflow-visible"
            >
              {layout.edges.map((edge) => {
                const incident =
                  selectedId === edge.from || selectedId === edge.to;
                return (
                  <path
                    key={`${edge.from}->${edge.to}`}
                    d={edge.path}
                    fill="none"
                    strokeWidth={incident ? 2 : 1.5}
                    className={
                      incident
                        ? 'stroke-cyan-400'
                        : edge.met
                          ? 'stroke-emerald-500/45'
                          : 'stroke-slate-700'
                    }
                  />
                );
              })}
            </svg>

            {layout.nodes.map((node) => {
              const view = index.get(node.id);
              if (!view) return null;
              return (
                <TicketGraphNode
                  key={node.id}
                  view={view}
                  index={index}
                  tone={ticketTone(view, index)}
                  selected={selectedId === node.id}
                  x={node.x}
                  y={node.y}
                  onSelect={() => onSelect(node.id)}
                />
              );
            })}
          </div>
        </div>
      </div>

      <div className="pointer-events-none absolute inset-0">
        <div className="pointer-events-auto absolute bottom-4 left-4 flex items-center gap-3 rounded-full border border-white/5 bg-slate-900/90 px-3 py-1.5 text-[10px] backdrop-blur-md">
          {LEGEND.map((entry) => (
            <span
              key={entry.label}
              className="flex items-center gap-1.5 text-slate-400"
            >
              <span
                aria-hidden="true"
                className={`h-1.5 w-1.5 rounded-full ${entry.dot}`}
              />
              {entry.label}
            </span>
          ))}
        </div>

        <CanvasZoomControls
          viewport={viewport}
          className="pointer-events-auto absolute right-4 bottom-4"
        />
      </div>
    </div>
  );
}

export default TicketGraph;
