/**
 * Pure graph queries over a schema-v2 definition, shared by the canvas
 * surfaces. Kept separate from `flowGraph` (which is about *rendering*) so the
 * traversals can be unit-tested on their own.
 */
import type { WorkflowDefinitionV2 } from './types';

/** Forward adjacency (`from` → `to[]`) for a definition's edges. */
function adjacency(def: WorkflowDefinitionV2): Map<string, string[]> {
  const adj = new Map<string, string[]>();
  for (const e of def.edges) {
    const list = adj.get(e.from);
    if (list) list.push(e.to);
    else adj.set(e.from, [e.to]);
  }
  return adj;
}

/**
 * The node ids strictly downstream of `nodeId` (its descendants) reached via
 * forward edges — excludes `nodeId` itself. The `seen` set makes it safe even
 * if a malformed definition contains a cycle (real v2 defs are acyclic).
 */
export function descendantIds(def: WorkflowDefinitionV2, nodeId: string): Set<string> {
  const adj = adjacency(def);
  const seen = new Set<string>();
  const stack = [...(adj.get(nodeId) ?? [])];
  while (stack.length) {
    const cur = stack.pop()!;
    if (seen.has(cur)) continue;
    seen.add(cur);
    for (const next of adj.get(cur) ?? []) {
      if (!seen.has(next)) stack.push(next);
    }
  }
  return seen;
}

/**
 * The **replay cone** for a node: the node itself plus everything downstream —
 * exactly the set `replay_from_step` re-executes (`replay_steps_from(..,
 * include_target: true)`). Powers the pre-confirm canvas highlight (P2.4).
 */
export function replayCone(def: WorkflowDefinitionV2, nodeId: string): Set<string> {
  const cone = descendantIds(def, nodeId);
  cone.add(nodeId);
  return cone;
}
