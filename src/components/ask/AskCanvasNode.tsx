/**
 * The card drawn for one Ask Canvas node — same 202×72 anatomy as
 * `WorkflowNode.tsx` so a diagram reads the same whichever surface drew it
 * (docs/ask-canvas/probe/Nodes.html). No placement math or citation
 * matching here; both are pure functions this component only consumes.
 */
import { Bot, type LucideIcon, UserCheck, Workflow, Boxes } from 'lucide-react';
import type { CanvasNode, NodeRole } from '../../types';

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

function resolveState(resolved: boolean, selected: boolean, cited: boolean): NodeCardState {
  if (!resolved) return 'unresolved';
  if (selected) return 'selected';
  if (cited) return 'cited';
  return 'resting';
}

export interface AskCanvasNodeProps {
  node: CanvasNode;
  /** The caller's looked-up `CanvasPathVerdict.resolved` for this node's
   *  `(id, path)` pair — this component never looks up the verdict itself. */
  resolved: boolean;
  selected: boolean;
  cited: boolean;
  onActivate: (id: string) => void;
}

export function AskCanvasNode({ node, resolved, selected, cited, onActivate }: AskCanvasNodeProps) {
  const state = resolveState(resolved, selected, cited);
  const Icon = ROLE_ICON[node.role];
  const clickable = resolved;

  return (
    <div
      data-state={state}
      className={[
        'flex h-[72px] w-[202px] flex-col justify-center gap-1.5 rounded-xl border px-3.5',
        'bg-slate-900/70 backdrop-blur-sm',
        clickable ? 'cursor-pointer' : '',
        STATE_CARD[state],
      ].join(' ')}
      {...(clickable ? { onClick: () => onActivate(node.id) } : {})}
    >
      <div className="flex min-w-0 items-center gap-3">
        <div
          className={[
            'flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border',
            ROLE_CHIP[node.role],
          ].join(' ')}
        >
          <Icon className="h-4 w-4" aria-hidden />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13px] font-medium text-slate-100" title={node.title}>
            {node.title}
          </div>
          <div className={`text-[9px] font-medium uppercase tracking-wide ${ROLE_TEXT[node.role]}`}>
            {ROLE_LABEL[node.role]}
          </div>
        </div>
      </div>
      {node.path !== null && (
        <div className="truncate pl-11 font-mono text-[9px] text-slate-500">{node.path}</div>
      )}
    </div>
  );
}
