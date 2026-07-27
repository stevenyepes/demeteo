/**
 * A compact, non-interactive picture of a workflow's shape (task P3.6, PRD
 * §6.3: *"`StartFeatureModal`'s per-step override list gains a mini-graph
 * preview"*).
 *
 * Deliberately **not** `WorkflowCanvas`. The canvas is the right thing for a
 * surface you navigate — it brings React Flow, an elk layout worker, pan/zoom
 * and a minimap, all of which are cost and none of which are usable inside a
 * 160px strip in a modal. What the launcher needs answered is one question:
 * *"is this the pipeline I meant — does it have the gate / the branch?"* So
 * this renders rank by rank, from the same v2 definition the canvas would.
 *
 * Ranks are longest-path depth, so a node always sits below everything it
 * depends on and a fan-out reads as a wider row. Nodes on the same rank have
 * no dependency between them — which is exactly the structural fact a chain
 * cannot show and the reason this preview exists.
 */
import { nodeTypeMeta } from './types';
import type { WorkflowDefinitionV2 } from './types';

export interface MiniGraphProps {
  definition: WorkflowDefinitionV2;
  className?: string;
}

/**
 * Group node ids into ranks by longest-path depth. Nodes whose dependencies
 * can't be resolved (a cycle — which lint refuses, but this must still render)
 * land in the last rank rather than vanishing.
 */
export function ranksOf(definition: WorkflowDefinitionV2): string[][] {
  const ids = definition.nodes.map((n) => n.id);
  const incoming = new Map<string, string[]>(ids.map((id) => [id, []]));
  for (const edge of definition.edges) {
    if (incoming.has(edge.to) && incoming.has(edge.from)) {
      incoming.get(edge.to)!.push(edge.from);
    }
  }

  const depth = new Map<string, number>();
  // Repeated relaxation: cheap at these sizes (tens of nodes) and needs no
  // topological sort of its own, so a malformed graph still terminates.
  for (let pass = 0; pass < ids.length; pass += 1) {
    let changed = false;
    for (const id of ids) {
      const deps = incoming.get(id) ?? [];
      const next = deps.length === 0 ? 0 : Math.max(...deps.map((d) => (depth.get(d) ?? 0) + 1));
      if (next !== (depth.get(id) ?? 0)) {
        depth.set(id, next);
        changed = true;
      }
    }
    if (!changed) break;
  }

  const ranks: string[][] = [];
  for (const id of ids) {
    const d = depth.get(id) ?? 0;
    while (ranks.length <= d) ranks.push([]);
    ranks[d].push(id);
  }
  return ranks.filter((rank) => rank.length > 0);
}

export function MiniGraph({ definition, className = '' }: MiniGraphProps) {
  const ranks = ranksOf(definition);
  const byId = new Map(definition.nodes.map((n) => [n.id, n]));

  if (definition.nodes.length === 0) {
    return (
      <div className={`text-[11px] text-slate-500 ${className}`} data-testid="mini-graph">
        This workflow has no steps.
      </div>
    );
  }

  return (
    <div
      className={`flex flex-col items-center gap-1 ${className}`}
      data-testid="mini-graph"
      aria-label="Workflow shape"
    >
      {ranks.map((rank, rankIndex) => (
        <div key={rankIndex} className="flex w-full flex-col items-center gap-1">
          {rankIndex > 0 && <span className="h-2 w-px bg-white/10" aria-hidden />}
          <div className="flex flex-wrap items-center justify-center gap-1.5">
            {rank.map((id) => {
              const node = byId.get(id)!;
              const meta = nodeTypeMeta(node.type);
              const Icon = meta.icon;
              return (
                <span
                  key={id}
                  title={`${node.title} · ${meta.label}`}
                  data-testid={`mini-node-${id}`}
                  className="flex max-w-[150px] items-center gap-1.5 rounded border border-white/10 bg-white/[0.03] px-2 py-1"
                >
                  <Icon className="h-3 w-3 shrink-0 text-slate-400" />
                  <span className="truncate text-[10px] text-slate-300">{node.title}</span>
                </span>
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}
