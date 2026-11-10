import { type Guard, getJson } from './api';
import { isWireWorkflow, type WireWorkflow, type WorkflowChoice, workflowOf } from './fleet';

/**
 * A Workflow this Hub offers: one of the desktop's bundled starters, embedded
 * in the Hub at build time. It can be chosen, never edited, and the browser is
 * sent its graph only, never its prompts.
 */
export type HubWorkflow = WorkflowChoice & { description: string };

type WireHubWorkflow = WireWorkflow & { description: string };

/** Ordered by name, as the Hub sends them. */
export async function listWorkflows(): Promise<HubWorkflow[]> {
  const wire = await getJson('/v1/workflows', isWireWorkflows);
  return wire.workflows.map((workflow) => ({ ...workflowOf(workflow), description: workflow.description }));
}

const isWireHubWorkflow: Guard<WireHubWorkflow> = (value): value is WireHubWorkflow =>
  isWireWorkflow(value) && typeof Reflect.get(value, 'description') === 'string';

const isWireWorkflows: Guard<{ workflows: WireHubWorkflow[] }> = (
  value,
): value is { workflows: WireHubWorkflow[] } => {
  const workflows = typeof value === 'object' && value !== null ? Reflect.get(value, 'workflows') : undefined;
  return Array.isArray(workflows) && workflows.every(isWireHubWorkflow);
};
