/**
 * The Ask Canvas: a lane band per declared lane, a stage column per declared
 * stage, and the node cards/edges `layoutAskCanvas` places across them. No
 * placement math or citation matching here — both are pure functions from
 * `src/lib/` this component only consumes (AGENTS.md §3's hexagon-boundary
 * rule applied to the frontend).
 *
 * Shaped like `TicketGraph.tsx`: an edges-only SVG under absolutely
 * positioned cards, never cards inside the SVG. An earlier revision read the
 * spec's "position from an SVG presentation attribute, never an inline style
 * prop" as a reason to put every card in a `<foreignObject>` inside a
 * translated `<g>`. The webview renders that subtree without the ancestor
 * transform and at a different scale, so every card landed in the wrong
 * column and the edges — correctly placed — appeared to point at nothing.
 * jsdom lays out no SVG, so no test could see it. The rule was about design
 * tokens; a computed coordinate was never one.
 */
import { useMemo, useRef, useState } from 'react';

import { citedNodeIds } from '../../lib/askCanvasCitations';
import { layoutAskCanvas } from '../../lib/askCanvasLayout';
import type { AskCanvas, CanvasPathVerdict, EdgeKind, NodeRole } from '../../types';
import { AskCanvasNode, ROLE_LABEL, type NodePathState } from './AskCanvasNode';

const HEADER_H = 32;
const LANE_LABEL_H = 22;

const ZOOM_MIN = 0.4;
const ZOOM_MAX = 1.5;
const ZOOM_STEP = 0.15;

/** Stroke treatment per edge kind — colour and dash. `kind` reaches the
 *  stroke and nothing else; the route is `askCanvasLayout`'s, derived from
 *  the two positions, so a mislabelled edge costs a colour and not a
 *  readable diagram. */
const EDGE_CLASS: Record<EdgeKind, string> = {
  hands_off: 'stroke-cyan-400/70',
  goes_back: 'stroke-violet-400/70 [stroke-dasharray:6_4]',
};

const EDGE_MARKER: Record<EdgeKind, string> = {
  hands_off: 'ask-canvas-arrow-hands-off',
  goes_back: 'ask-canvas-arrow-goes-back',
};

const MARKER_FILL: Record<EdgeKind, string> = {
  hands_off: 'fill-cyan-400/70',
  goes_back: 'fill-violet-400/70',
};

const LEGEND: readonly NodeRole[] = ['orchestration', 'boundary', 'agent', 'needs_human'];

const LEGEND_DOT: Record<NodeRole, string> = {
  orchestration: 'bg-violet-400',
  boundary: 'bg-cyan-400',
  agent: 'bg-emerald-400',
  needs_human: 'bg-amber-400',
};

export interface AskCanvasViewProps {
  canvas: AskCanvas;
  answerText: string;
  canvasPaths: CanvasPathVerdict[];
  selectedNodeId: string | null;
  onActivate: (id: string) => void;
}

/** Verdicts are looked up by `(node_id, path)`, not `node_id` alone — a
 *  verdict computed against a stale `path` must not be credited to a node
 *  whose path has since changed. */
function verdictKey(nodeId: string, path: string): string {
  return `${nodeId} ${path}`;
}

export function AskCanvasView({
  canvas,
  answerText,
  canvasPaths,
  selectedNodeId,
  onActivate,
}: AskCanvasViewProps) {
  const layout = useMemo(() => layoutAskCanvas(canvas), [canvas]);
  const citedIds = useMemo(() => citedNodeIds(answerText, canvas.nodes), [answerText, canvas.nodes]);

  const positionsById = useMemo(
    () => new Map(layout.nodes.map((node) => [node.id, node])),
    [layout.nodes],
  );

  const verdictsByKey = useMemo(
    () => new Map(canvasPaths.map((verdict) => [verdictKey(verdict.node_id, verdict.path), verdict])),
    [canvasPaths],
  );

  const viewport = useRef<HTMLDivElement | null>(null);
  const [zoom, setZoom] = useState(1);

  function fit() {
    const element = viewport.current;
    if (!element || layout.width === 0 || layout.height === 0) return;
    setZoom(clamp(Math.min(element.clientWidth / layout.width, element.clientHeight / layout.height)));
  }

  return (
    <div data-testid="ask-canvas-view" className="relative flex h-full w-full flex-col">
      <div className="flex shrink-0 items-baseline gap-2 px-4 pt-3 pb-2">
        <h2 className="truncate font-heading text-sm font-semibold text-slate-100">{canvas.title}</h2>
      </div>

      <div className="relative min-h-0 flex-1">
        <div
          ref={viewport}
          className="absolute inset-0 overflow-auto bg-[radial-gradient(rgba(255,255,255,0.05)_1px,transparent_0)] bg-[length:20px_20px]"
        >
          <div style={{ width: layout.width * zoom, height: (HEADER_H + layout.height) * zoom }}>
            <div
              className="relative origin-top-left"
              style={{
                width: layout.width,
                height: HEADER_H + layout.height,
                transform: `scale(${zoom})`,
              }}
            >
              {layout.columns.map((column) => (
                <div
                  key={`stage-${column.stage}`}
                  style={{ left: column.x, top: 0, width: column.width, height: HEADER_H }}
                  className="absolute flex items-center truncate px-2 font-heading text-[11px] font-medium uppercase tracking-wide text-slate-400"
                >
                  {canvas.stages[column.stage]}
                </div>
              ))}

              <div className="absolute inset-x-0" style={{ top: HEADER_H, height: layout.height }}>
                <svg
                  aria-label={canvas.title}
                  role="img"
                  width={layout.width}
                  height={layout.height}
                  className="pointer-events-none absolute inset-0 overflow-visible"
                >
                  <defs>
                    {(Object.keys(EDGE_MARKER) as EdgeKind[]).map((kind) => (
                      <marker
                        key={kind}
                        id={EDGE_MARKER[kind]}
                        viewBox="0 0 10 10"
                        refX="9"
                        refY="5"
                        markerWidth="5"
                        markerHeight="5"
                        orient="auto-start-reverse"
                      >
                        <path d="M0 0 L10 5 L0 10 z" className={MARKER_FILL[kind]} />
                      </marker>
                    ))}
                  </defs>

                  {layout.bands.map((band) => (
                    <rect
                      key={`band-${band.lane}`}
                      data-testid="ask-canvas-band"
                      data-lane={band.lane}
                      x={0}
                      y={band.y}
                      width={layout.width}
                      height={band.height}
                      rx={12}
                      className="fill-slate-900/30 stroke-slate-800/70"
                    />
                  ))}

                  <g data-testid="ask-canvas-edge-layer">
                    {layout.edges.map((edge) => (
                      <path
                        key={`${edge.from}->${edge.to}->${edge.kind}`}
                        d={edge.path}
                        fill="none"
                        strokeWidth={1.5}
                        strokeLinejoin="round"
                        markerEnd={`url(#${EDGE_MARKER[edge.kind]})`}
                        className={EDGE_CLASS[edge.kind]}
                      />
                    ))}
                  </g>
                </svg>

                {layout.bands.map((band) => (
                  <div
                    key={`lane-label-${band.lane}`}
                    style={{ left: 12, top: band.y + 4, height: LANE_LABEL_H }}
                    className="pointer-events-none absolute flex items-center px-2 font-mono text-[10px] uppercase tracking-[0.12em] text-slate-500"
                  >
                    {canvas.lanes[band.lane]}
                  </div>
                ))}

                {canvas.nodes.map((node) => {
                  const position = positionsById.get(node.id);
                  if (!position) return null;
                  return (
                    <AskCanvasNode
                      key={node.id}
                      node={node}
                      pathState={pathStateOf(node.id, node.path, verdictsByKey)}
                      selected={selectedNodeId === node.id}
                      cited={citedIds.has(node.id)}
                      x={position.x}
                      y={position.y}
                      onActivate={onActivate}
                    />
                  );
                })}
              </div>
            </div>
          </div>
        </div>

        <div className="pointer-events-none absolute inset-0">
          <div className="pointer-events-auto absolute bottom-4 left-4 flex items-center gap-3 rounded-full border border-white/5 bg-slate-900/90 px-3 py-1.5 text-[10px] backdrop-blur-md">
            {LEGEND.map((role) => (
              <span key={role} className="flex items-center gap-1.5 text-slate-400">
                <span aria-hidden="true" className={`h-1.5 w-1.5 rounded-full ${LEGEND_DOT[role]}`} />
                {ROLE_LABEL[role]}
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
    </div>
  );
}

/** A node that never named a file is `none`, not `missing` — see
 *  [`NodePathState`](./AskCanvasNode.tsx). */
function pathStateOf(
  nodeId: string,
  path: string | null,
  verdicts: ReadonlyMap<string, CanvasPathVerdict>,
): NodePathState {
  if (path === null) return 'none';
  return verdicts.get(verdictKey(nodeId, path))?.resolved === true ? 'resolved' : 'missing';
}

function clamp(zoom: number): number {
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom));
}

export default AskCanvasView;
