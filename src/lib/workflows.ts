import { invoke } from "@tauri-apps/api/core";
import type { WorkflowSchedule, WorkflowWithSteps } from "../types";
import type { WorkflowDefinitionV2 } from "../components/canvas/types";
import type { NodeTypeInfo } from "../components/canvas/nodeCatalog";
import type { LintFinding } from "../components/canvas/lint";

export async function listWorkflows(): Promise<WorkflowWithSteps[]> {
  return invoke<WorkflowWithSteps[]>("workflow_list");
}

export async function getWorkflow(workflowId: string): Promise<WorkflowWithSteps> {
  return invoke<WorkflowWithSteps>("workflow_get", { workflowId });
}

/**
 * Store a schema-v2 graph as a new version. `workflowId === null` mints the
 * workflow row, so an abandoned "new workflow" never leaves a husk behind.
 * `note` is what the version drawer lists beside the row.
 */
export async function saveWorkflow(input: {
  workflowId: string | null;
  name: string;
  description: string;
  definition: WorkflowDefinitionV2;
  note: string | null;
}): Promise<WorkflowWithSteps> {
  return invoke<WorkflowWithSteps>("workflow_save", {
    workflowId: input.workflowId,
    name: input.name,
    description: input.description,
    definition: input.definition,
    note: input.note,
  });
}

export async function deleteWorkflow(workflowId: string): Promise<void> {
  return invoke<void>("workflow_delete", { workflowId });
}

/** Serde shape of the Rust `WorkflowVersion` row. */
export interface WorkflowVersionRow {
  id: string;
  workflow_id: string;
  version: number;
  steps_json: string;
  note: string | null;
  created_at: number;
}

export async function listWorkflowVersions(workflowId: string): Promise<WorkflowVersionRow[]> {
  return invoke<WorkflowVersionRow[]>("workflow_versions", { workflowId });
}

/** The schema-v2 graph for one stored version. Migration is Rust-only, so
 *  the drawer cannot derive this from the `steps_json` string
 *  {@link listWorkflowVersions} hands it. */
export async function workflowVersionGraph(
  workflowId: string,
  versionId: string,
): Promise<WorkflowDefinitionV2> {
  return invoke<WorkflowDefinitionV2>("workflow_version_graph", { workflowId, versionId });
}

/** Copy a stored version forward as a **new** version. History only grows —
 *  the row it copies stays where it was. */
export async function restoreWorkflowVersion(
  workflowId: string,
  versionId: string,
): Promise<WorkflowWithSteps> {
  return invoke<WorkflowWithSteps>("workflow_restore_version", { workflowId, versionId });
}

/** Starter-pack workflows only; appends a version like a restore does. */
export async function revertWorkflowToDefault(workflowId: string): Promise<WorkflowWithSteps> {
  return invoke<WorkflowWithSteps>("workflow_revert_to_default", { workflowId });
}

/** Pretty-printed schema-v2 document, with the workflow's `description`
 *  alongside the graph (the v2 schema has no place for it). */
export async function exportWorkflow(workflowId: string): Promise<string> {
  return invoke<string>("workflow_export", { workflowId });
}

/** Import a workflow document of either schema version; a v1 steps list
 *  migrates on the way in. The workflow always gets a fresh id. */
export async function importWorkflow(json: string): Promise<WorkflowWithSteps> {
  return invoke<WorkflowWithSteps>("workflow_import", { json });
}

/** The builder's palette entries, projected from the Rust `NodeTypeRegistry`.
 *  Static for a given build — {@link ../components/canvas/nodeCatalog} is what
 *  caches it; this is the bare round-trip. */
export async function listNodeTypes(): Promise<NodeTypeInfo[]> {
  return invoke<NodeTypeInfo[]>("node_types_list");
}

/** Structural lint for a schema-v2 graph. `definition` is the parsed
 *  document, not a string: the command takes `serde_json::Value`. */
export async function lintWorkflow(definition: unknown): Promise<LintFinding[]> {
  return invoke<LintFinding[]>("workflow_lint", { definition });
}

/** Write (or clear, with `null`) the workflow's cron schedule. `next_run_at`
 *  is recomputed backend-side, so a stale one cannot pin a schedule to a time
 *  that has already passed. */
export async function saveWorkflowSchedule(
  workflowId: string,
  schedule: WorkflowSchedule | null,
): Promise<void> {
  return invoke<void>("workflow_save_schedule", { workflowId, schedule });
}
