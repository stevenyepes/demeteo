/**
 * Workflow badge lookup for the "Active Running Pipelines" cards in
 * `ProjectHome`.
 *
 * Two regressions this guards against:
 *
 *  1. A feature whose `workflow_id` is null/empty used to render a violet badge
 *     with the literal word "undefined" in it, looking like a real match. Any
 *     miss must fall through to the muted "unknown" badge.
 *
 *  2. The lookup once matched positionally rather than by id, so deleting a
 *     workflow (or a feature outliving its workflow) silently relabelled cards
 *     with the wrong name. The map is keyed by `workflow.id`, exactly.
 */

export interface WorkflowMeta {
  name: string;
  is_starter: boolean;
}

export type WorkflowBadge =
  | { variant: 'fallback' }
  | { variant: 'known'; name: string; is_starter: boolean };

/**
 * Build the `workflow_id → meta` lookup. Entries without a usable string id are
 * dropped rather than admitted with a junk key.
 */
export function buildWorkflowById(
  list: ReadonlyArray<{ id?: unknown; name?: unknown; is_starter?: unknown } | null | undefined>,
): Map<string, WorkflowMeta> {
  const lookup = new Map<string, WorkflowMeta>();

  for (const wf of list) {
    if (wf && typeof wf.id === 'string' && wf.id.length > 0) {
      lookup.set(wf.id, {
        name: typeof wf.name === 'string' ? wf.name : '',
        is_starter: Boolean(wf.is_starter),
      });
    }
  }

  return lookup;
}

/** Classify a feature's workflow badge. A miss is always the muted fallback. */
export function classifyWorkflowBadge(
  feature: { workflow_id?: string | null },
  lookup: ReadonlyMap<string, WorkflowMeta>,
): WorkflowBadge {
  const meta = feature?.workflow_id ? lookup.get(feature.workflow_id) : undefined;
  if (!meta) return { variant: 'fallback' };

  return { variant: 'known', name: meta.name, is_starter: meta.is_starter };
}
