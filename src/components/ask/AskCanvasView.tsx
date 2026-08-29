/**
 * The Ask Canvas grid: stage columns, lane rows, one background cell per
 * declared `(stage, lane)` pair, and the node cards/edges `layoutAskCanvas`
 * places inside them. No placement math or citation matching here — both are
 * pure functions from `src/lib/` this component only consumes (AGENTS.md
 * §3's hexagon-boundary rule applied to the frontend).
 *
 * Every position comes from an SVG presentation attribute (`x`/`y`/`width`/
 * `height`/`transform`), never an inline style prop — this file is inside the
 * grep the diff review runs by hand against Acceptance Criterion 7.
 */
import { useMemo } from 'react';

import { citedNodeIds } from '../../lib/askCanvasCitations';
import { layoutAskCanvas, NODE_H, NODE_W } from '../../lib/askCanvasLayout';
import type { AskCanvas, CanvasNode, EdgeKind } from '../../types';
import { AskCanvasNode } from './AskCanvasNode';

const HEADER_H = 36;
const LANE_LABEL_W = 120;

/** Stroke treatment per edge kind — color and dash, mirroring the
 *  `Record<Role, string>` lookup-table pattern used across this feature. */
const EDGE_CLASS: Record<EdgeKind, string> = {
  hands_off: 'stroke-cyan-400/70',
  goes_back: 'stroke-violet-400/70 [stroke-dasharray:6_4]',
};

export interface AskCanvasViewProps {
  canvas: AskCanvas;
  answerText: string;
  selectedNodeId: string | null;
  onActivate: (id: string) => void;
}

export function AskCanvasView({ canvas, answerText, selectedNodeId, onActivate }: AskCanvasViewProps) {
  const layout = useMemo(() => layoutAskCanvas(canvas), [canvas]);
  const citedIds = useMemo(() => citedNodeIds(answerText, canvas.nodes), [answerText, canvas.nodes]);

  const positionsById = useMemo(() => new Map(layout.nodes.map((node) => [node.id, node])), [layout.nodes]);

  const nodesByCell = useMemo(() => {
    const map = new Map<string, CanvasNode[]>();
    for (const node of canvas.nodes) {
      const key = `${node.stage}:${node.lane}`;
      const occupants = map.get(key);
      if (occupants) {
        occupants.push(node);
      } else {
        map.set(key, [node]);
      }
    }
    return map;
  }, [canvas.nodes]);

  const stageHeaderCells = layout.cells.filter((cell) => cell.lane === 0);
  const laneLabelCells = layout.cells.filter((cell) => cell.stage === 0);

  return (
    <div data-testid="ask-canvas-view" className="relative h-full w-full overflow-auto">
      <svg
        role="img"
        aria-label={canvas.title}
        width={LANE_LABEL_W + layout.width}
        height={HEADER_H + layout.height}
      >
        {stageHeaderCells.map((cell) => (
          <foreignObject
            key={`stage-header-${cell.stage}`}
            x={LANE_LABEL_W + cell.x}
            y={0}
            width={cell.width}
            height={HEADER_H}
          >
            <div className="flex h-full items-center truncate px-2 font-heading text-[11px] font-medium uppercase tracking-wide text-slate-400">
              {canvas.stages[cell.stage]}
            </div>
          </foreignObject>
        ))}

        {laneLabelCells.map((cell) => (
          <foreignObject
            key={`lane-label-${cell.lane}`}
            x={0}
            y={HEADER_H + cell.y}
            width={LANE_LABEL_W}
            height={cell.height}
          >
            <div className="flex h-full items-center truncate px-2 text-[11px] font-medium text-slate-400">
              {canvas.lanes[cell.lane]}
            </div>
          </foreignObject>
        ))}

        <g transform={`translate(${LANE_LABEL_W}, ${HEADER_H})`}>
          {layout.cells.map((cell) => {
            const occupied = (nodesByCell.get(`${cell.stage}:${cell.lane}`) ?? []).length > 0;
            return (
              <rect
                key={`cell-${cell.stage}:${cell.lane}`}
                data-testid={occupied ? undefined : 'ask-canvas-empty-cell'}
                x={cell.x}
                y={cell.y}
                width={cell.width}
                height={cell.height}
                rx={12}
                className="fill-slate-900/30 stroke-slate-800"
              />
            );
          })}

          <g data-testid="ask-canvas-edge-layer">
            {layout.edges.map((edge) => (
              <path
                key={`${edge.from}->${edge.to}`}
                d={edge.path}
                fill="none"
                strokeWidth={1.5}
                className={EDGE_CLASS[edge.kind]}
              />
            ))}
          </g>

          {canvas.nodes.map((node) => {
            const position = positionsById.get(node.id);
            if (!position) return null;
            return (
              <foreignObject key={node.id} x={position.x} y={position.y} width={NODE_W} height={NODE_H}>
                <AskCanvasNode
                  node={node}
                  selected={selectedNodeId === node.id}
                  cited={citedIds.has(node.id)}
                  onActivate={onActivate}
                />
              </foreignObject>
            );
          })}
        </g>
      </svg>
    </div>
  );
}

export default AskCanvasView;
