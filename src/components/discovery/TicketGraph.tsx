import React, { useMemo, useRef, useState } from 'react';

import { layoutTicketGraph } from '../../lib/ticketGraphLayout';
import { ticketTone, type TicketIndex } from '../../lib/ticketPresentation';
import type { TicketView } from '../../types';
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

const ZOOM_MIN = 0.4;
const ZOOM_MAX = 1.5;
const ZOOM_STEP = 0.15;

/**
 * What depends on what.
 *
 * **Not `WorkflowCanvas`** — `docs/TASKS_DISCOVERY.md` records why: reuse would
 * import the run-tone vocabulary the canvas exists to paint, and a ticket lane
 * is not a run status. With no pan, no wheel zoom, no drag and no minimap
 * (§6.6), React Flow and its layout worker would be cost with no payer.
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
  const viewport = useRef<HTMLDivElement | null>(null);
  const [zoom, setZoom] = useState(1);

  function fit() {
    const element = viewport.current;
    if (!element || layout.width === 0 || layout.height === 0) return;
    setZoom(
      clamp(
        Math.min(
          element.clientWidth / layout.width,
          element.clientHeight / layout.height,
        ),
      ),
    );
  }

  return (
    <div data-testid="ticket-graph" className="absolute inset-0">
      <div
        ref={viewport}
        className="absolute inset-0 overflow-auto bg-[#050608] bg-[radial-gradient(#334155_1px,transparent_1px)] bg-[length:20px_20px]"
      >
        <div
          style={{
            width: layout.width * zoom,
            height: layout.height * zoom,
          }}
        >
          <div
            className="relative origin-top-left"
            style={{
              width: layout.width,
              height: layout.height,
              transform: `scale(${zoom})`,
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

        <div className="pointer-events-auto absolute right-4 bottom-4 flex items-center gap-1.5">
          <button
            type="button"
            aria-label="Zoom out"
            onClick={() => setZoom((current) => clamp(current - ZOOM_STEP))}
            className="btn-secondary bg-slate-900/90! px-2.5! py-1.5!"
          >
            &minus;
          </button>
          <button
            type="button"
            aria-label="Zoom in"
            onClick={() => setZoom((current) => clamp(current + ZOOM_STEP))}
            className="btn-secondary bg-slate-900/90! px-2.5! py-1.5!"
          >
            +
          </button>
          <button
            type="button"
            onClick={fit}
            className="btn-secondary bg-slate-900/90! px-2.5! py-1.5! text-[11px]"
          >
            Fit
          </button>
        </div>
      </div>
    </div>
  );
}

function clamp(zoom: number): number {
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom));
}

export default TicketGraph;
