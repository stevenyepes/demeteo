import { ranksOf } from '../components/canvas/MiniGraph';
import type { TicketView } from '../types';

/**
 * Where every ticket node sits, and the curve between each pair.
 *
 * The mock's `x`/`y` literals are fixture values rather than a design
 * (`docs/TASKS_DISCOVERY.md`, "The ticket graph is its own component"), so the
 * only structural fact worth keeping from them is the shape: a node sits below
 * everything it depends on, and a fan-out reads as a wider row. `ranksOf` is
 * exactly that computation and is already cycle-tolerant, which matters here
 * because a cycle is refused at decompose time but a stored plan predating
 * that check must still render.
 */

export const NODE_W = 280;
/** Fixed rather than measured: the edges anchor to the node's bottom edge, and
 *  a curve drawn from a height the browser has not laid out yet points at
 *  nothing. Nodes clamp their content to it. */
export const NODE_H = 96;
const COL_GAP = 20;
const ROW_GAP = 60;
const PAD = 16;

export interface GraphNode {
  id: string;
  x: number;
  y: number;
}

export interface GraphEdge {
  from: string;
  to: string;
  /** The prerequisite has released this dependent — emerald rather than slate. */
  met: boolean;
  path: string;
}

export interface GraphLayout {
  nodes: GraphNode[];
  edges: GraphEdge[];
  width: number;
  height: number;
}

export function layoutTicketGraph(tickets: readonly TicketView[]): GraphLayout {
  const present = new Set(tickets.map((view) => view.ticket.id));
  const pairs = tickets.flatMap((view) =>
    view.ticket.blocked_by
      .filter((id) => present.has(id))
      .map((id) => ({ from: id, to: view.ticket.id })),
  );

  const ranks = ranksOf({
    schema_version: 2,
    id: 'ticket-graph',
    name: 'ticket-graph',
    nodes: tickets.map((view) => ({ id: view.ticket.id, type: 'agent', title: view.ticket.title })),
    edges: pairs.map((pair) => ({ from: pair.from, to: pair.to })),
  });

  const widest = ranks.reduce((max, rank) => Math.max(max, rank.length), 0);
  const width = PAD * 2 + widest * NODE_W + Math.max(0, widest - 1) * COL_GAP;
  const height = PAD * 2 + ranks.length * NODE_H + Math.max(0, ranks.length - 1) * ROW_GAP;

  const placed = new Map<string, GraphNode>();
  ranks.forEach((rank, row) => {
    const rowWidth = rank.length * NODE_W + (rank.length - 1) * COL_GAP;
    const left = (width - rowWidth) / 2;
    rank.forEach((id, column) => {
      placed.set(id, {
        id,
        x: left + column * (NODE_W + COL_GAP),
        y: PAD + row * (NODE_H + ROW_GAP),
      });
    });
  });

  const outstanding = new Map(
    tickets.map((view) => [
      view.ticket.id,
      new Set(view.standing.blockers.map((blocker) => blocker.id)),
    ]),
  );

  const edges: GraphEdge[] = [];
  for (const pair of pairs) {
    const from = placed.get(pair.from);
    const to = placed.get(pair.to);
    if (!from || !to) continue;
    edges.push({
      from: pair.from,
      to: pair.to,
      met: !outstanding.get(pair.to)?.has(pair.from),
      path: curve(from, to),
    });
  }

  return { nodes: [...placed.values()], edges, width, height };
}

function curve(from: GraphNode, to: GraphNode): string {
  const x1 = from.x + NODE_W / 2;
  const y1 = from.y + NODE_H;
  const x2 = to.x + NODE_W / 2;
  const y2 = to.y;
  const bend = Math.max(16, (y2 - y1) / 2);
  return `M${round(x1)} ${round(y1)} C${round(x1)} ${round(y1 + bend)}, ${round(x2)} ${round(y2 - bend)}, ${round(x2)} ${round(y2)}`;
}

function round(value: number): number {
  return Math.round(value * 100) / 100;
}
