/**
 * The card drawn for one Ask Canvas node — same 202×72 anatomy as
 * `WorkflowNode.tsx` so a diagram reads the same whichever surface drew it
 * (docs/ask-canvas/probe/Nodes.html). No placement math or citation
 * matching here; both are pure functions this component only consumes.
 *
 * Positioned by the caller through an inline `left`/`top`, the way
 * `TicketGraphNode.tsx` is and for the same reason — a computed coordinate is
 * a datum, not a design token, so AGENTS.md §4's ban does not reach it. This
 * card previously lived inside an SVG `<foreignObject>` to avoid that inline
 * style; under a transformed `<g>` the webview dropped the ancestor transform
 * and mis-scaled the subtree, which put every card in the wrong column.
 */
import { Bot, type LucideIcon, UserCheck, Workflow, Boxes } from 'lucide-react';
import type { CanvasNode, NodeRole } from '../../types';
import { NODE_H, NODE_W } from '../../lib/askCanvasLayout';

/**
 * What this node's `path` turned out to be, which is three answers and not
 * two. `none` is a node that never named a file — a person, a concept, every
 * `needs_human` node by definition — and it is a normal card. `missing` is a
 * node that named one which is not there, and only that earns the dimmed,
 * dashed treatment. Collapsing the two rendered every Gate as a ghost.
 */
export type NodePathState = 'none' | 'resolved' | 'missing';

type NodeCardState = 'resting' | 'selected' | 'cited' | 'unresolved';

export const ROLE_ICON: Record<NodeRole, LucideIcon> = {
  orchestration: Workflow,
  boundary: Boxes,
  agent: Bot,
  needs_human: UserCheck,
};

export const ROLE_LABEL: Record<NodeRole, string> = {
  orchestration: 'Orchestration',
  boundary: 'Boundary',
  agent: 'Running agent',
  needs_human: 'Needs a human',
};

/** Icon-chip tint per role — the app's colour language, unchanged by state. */
export const ROLE_CHIP: Record<NodeRole, string> = {
  orchestration: 'border-violet-500/20 bg-violet-500/10 text-violet-300',
  boundary: 'border-cyan-500/20 bg-cyan-500/10 text-cyan-300',
  agent: 'border-emerald-500/20 bg-emerald-500/10 text-emerald-300',
  needs_human: 'border-amber-500/20 bg-amber-500/10 text-amber-300',
};

export const ROLE_TEXT: Record<NodeRole, string> = {
  orchestration: 'text-violet-300',
  boundary: 'text-cyan-300',
  agent: 'text-emerald-300',
  needs_human: 'text-amber-300',
};

/** Card border + glow per state. `unresolved` is a muted/dashed overlay, not
 *  a ring — it never competes with the role tone carried by `ROLE_CHIP`. */
const STATE_CARD: Record<NodeCardState, string> = {
  resting: 'border-slate-700/60 shadow-lg shadow-black/30',
  selected:
    'border-cyan-400/70 shadow-[0_0_0_1px_rgba(34,211,238,0.4),0_0_18px_rgba(34,211,238,0.25)]',
  cited:
    'border-violet-400/60 shadow-[0_0_0_2px_rgba(167,139,250,0.45),0_0_18px_rgba(139,92,246,0.25)]',
  unresolved: 'border-slate-600/50 border-dashed opacity-50 shadow-none',
};

function resolveState(
  pathState: NodePathState,
  selected: boolean,
  cited: boolean,
): NodeCardState {
  if (pathState === 'missing') return 'unresolved';
  if (selected) return 'selected';
  if (cited) return 'cited';
  return 'resting';
}

/** The last two segments, which is the part that names the thing — the mock
 *  labels a node `step_executor/driver.rs`, not the repo path that reaches
 *  it. Truncating a full path from the right left every node in a Rust tree
 *  reading `crates/demeteo-core…`, which identifies nothing. */
export function pathTail(path: string): string {
  const segments = path.split('/').filter((segment) => segment.length > 0);
  return segments.slice(-2).join('/');
}

export interface AskCanvasNodeProps {
  node: CanvasNode;
  /** Derived by the caller from its `(id, path)` verdict lookup — this
   *  component never looks up a verdict itself. */
  pathState: NodePathState;
  selected: boolean;
  cited: boolean;
  x: number;
  y: number;
  onActivate: (id: string) => void;
}

export function AskCanvasNode({
  node,
  pathState,
  selected,
  cited,
  x,
  y,
  onActivate,
}: AskCanvasNodeProps) {
  const state = resolveState(pathState, selected, cited);
  const Icon = ROLE_ICON[node.role];
  const clickable = pathState !== 'missing';

  const body = (
    <>
      <div className="flex min-w-0 items-center gap-3">
        <div
          className={[
            'flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border',
            ROLE_CHIP[node.role],
          ].join(' ')}
        >
          <Icon className="h-4 w-4" aria-hidden />
        </div>
        <div className="min-w-0 flex-1 text-left">
          <div className="truncate text-[13px] font-medium text-slate-100" title={node.title}>
            {node.title}
          </div>
          <div className={`text-[9px] font-medium uppercase tracking-wide ${ROLE_TEXT[node.role]}`}>
            {ROLE_LABEL[node.role]}
          </div>
        </div>
      </div>
      {node.path !== null && (
        <div className="truncate pl-11 text-left font-mono text-[9px] text-slate-500" title={node.path}>
          {pathTail(node.path)}
        </div>
      )}
    </>
  );

  const shell = [
    'absolute flex flex-col justify-center gap-1.5 rounded-xl border px-3.5',
    'bg-slate-900/70 backdrop-blur-sm',
    STATE_CARD[state],
  ].join(' ');

  // A computed coordinate is the datum here, exactly as `TicketGraphNode.tsx`
  // records for its own inline style.
  const box = { left: x, top: y, width: NODE_W, height: NODE_H };

  if (!clickable) {
    return (
      <div data-state={state} style={box} className={shell}>
        {body}
      </div>
    );
  }

  return (
    <button
      type="button"
      data-state={state}
      aria-pressed={selected}
      onClick={() => onActivate(node.id)}
      style={box}
      className={shell}
    >
      {body}
    </button>
  );
}
