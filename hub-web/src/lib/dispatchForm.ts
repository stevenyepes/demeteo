import type { NodeConfigV2, WorkflowDefinitionV2 } from '../../../src/components/canvas/types';
import {
  EFFORT_SUPPORT,
  type EffortLevel,
  reconcileEffort,
  supportedEffortsFor,
} from '../../../src/lib/effortLevels';
import type { StepOverride } from '../../../src/types';
import type { DispatchRequest } from './dispatch';
import type { InstanceDetail, WorkflowChoice } from './fleet';

export type FieldResult<T> = { ok: true; value: T } | { ok: false; error: string };

/** One step's pins as the override panel holds them; blank or absent inherits. */
export type StepOverrideDraft = { agentKind?: string; model?: string; effort?: EffortLevel };

/** The form once its text fields have parsed: what `buildDispatchRequest` is handed. */
export type DispatchForm = {
  projectId: string;
  workflowId: string;
  title: string;
  description: string;
  /** Blank inherits the project's default. */
  agentKind: string;
  /** Blank inherits the project's default. */
  model: string;
  effort: EffortLevel | null;
  overrides: Record<string, StepOverrideDraft>;
  maxBudgetCents: number;
  maxWallClockSecs: number | undefined;
};

export type DispatchBlocker = 'no-read' | 'no-spend' | 'no-snapshot';

/** The fields a project's run-shape row fills in, as the form holds them. */
export type Prefill = {
  agentKind: string;
  model: string;
  effort: EffortLevel | '';
  workflowId: string;
  maxCostUsd: string;
};

export type PrefillField = keyof Prefill;

const DOLLARS = /^(\d+)(?:\.(\d{1,2}))?$/;
const WHOLE = /^\d+$/;

/**
 * The cap is required, unlike `src/lib/detachedCaps.ts` where blank means "no
 * cap": the instance refuses a Hub-started run with none. The digits are read
 * as integer and fraction parts, never `Number(text) * 100`, which turns
 * `"0.29"` into 28.999…; and `Number` would also take `"1e2"` and `"0x10"`.
 */
export function parseMaxCostCents(text: string): FieldResult<number> {
  const trimmed = text.trim();
  if (trimmed === '') {
    return { ok: false, error: 'A max cost is required: the instance refuses to start a run with no cap.' };
  }
  const match = DOLLARS.exec(trimmed);
  if (match === null) return { ok: false, error: 'Enter an amount in dollars, with at most two decimals.' };
  const cents = Number(match[1]) * 100 + Number((match[2] ?? '').padEnd(2, '0'));
  if (cents <= 0) return { ok: false, error: 'Enter an amount greater than 0.' };
  if (!Number.isSafeInteger(cents)) return { ok: false, error: 'This amount is too large.' };
  return { ok: true, value: cents };
}

/** The inverse of `parseMaxCostCents`; blank for any amount it would refuse, so the field stays required. */
export function usdOfCents(cents: number): string {
  if (!Number.isSafeInteger(cents) || cents <= 0) return '';
  return `${Math.floor(cents / 100)}.${String(cents % 100).padStart(2, '0')}`;
}

/**
 * What the form shows once `projectId` is chosen: each field in `edited`
 * keeps its `held` value, and the rest take the project's run-shape default,
 * or blank where it has none, so one project's defaults never carry over to
 * another's dispatch.
 *
 * A default the form cannot offer is left blank rather than selected: a
 * workflow the instance did not report would be refused as
 * `unknown_workflow`, and an agent kind this build has no option for would
 * leave the select showing something other than its value. The default model
 * belongs to the default agent, so it is not filled in under an agent the
 * user picked.
 */
export function prefillFor(
  instance: InstanceDetail,
  projectId: string,
  held: Prefill,
  edited: ReadonlySet<PrefillField>,
): Prefill {
  const row = instance.runShape?.find((entry) => entry.projectId === projectId);
  const workflows = instance.workflows ?? [];
  const defaultAgent = row?.agentKind && Object.keys(EFFORT_SUPPORT).includes(row.agentKind) ? row.agentKind : '';
  const agentKind = edited.has('agentKind') ? held.agentKind : defaultAgent;
  const keepModel = edited.has('model') || edited.has('agentKind');
  const workflowId = edited.has('workflowId')
    ? held.workflowId
    : (workflows.find((choice) => choice.workflowId === row?.workflowId) ?? workflows[0])?.workflowId ?? '';
  return {
    agentKind,
    model: keepModel ? held.model : (row?.model ?? ''),
    effort: reconcileEffort(edited.has('effort') ? held.effort : (row?.effort ?? ''), supportedEffortsFor(agentKind)),
    workflowId,
    maxCostUsd: edited.has('maxCostUsd') ? held.maxCostUsd : usdOfCents(row?.maxBudgetCents ?? 0),
  };
}

/** Blank is no wall-clock cap. Whole minutes, because the wire carries `max_wall_clock_secs: u64`. */
export function parseWallClockSecs(minutesText: string): FieldResult<number | undefined> {
  const trimmed = minutesText.trim();
  if (trimmed === '') return { ok: true, value: undefined };
  const secs = WHOLE.test(trimmed) ? Number(trimmed) * 60 : Number.NaN;
  if (!(secs > 0) || !Number.isSafeInteger(secs)) {
    return { ok: false, error: 'Enter a whole number of minutes above 0, or leave blank for no cap.' };
  }
  return { ok: true, value: secs };
}

/** A step with nothing pinned has no entry, never an all-unset one, as `StepOverride` requires. */
export function toStepOverrides(map: Record<string, StepOverrideDraft>): StepOverride[] {
  const overrides: StepOverride[] = [];
  for (const [stepId, draft] of Object.entries(map)) {
    const agentKind = draft.agentKind?.trim();
    const model = draft.model?.trim();
    if (!agentKind && !model && !draft.effort) continue;
    overrides.push({
      step_id: stepId,
      ...(agentKind ? { agent_kind: agentKind } : {}),
      ...(model ? { model } : {}),
      ...(draft.effort ? { effort: draft.effort } : {}),
    });
  }
  return overrides;
}

/** The node types whose agent the run resolves per step; a gate or finalize runs no agent to pin. */
export function isOverridable(nodeType: string): boolean {
  return nodeType === 'agent' || nodeType === 'sequence';
}

/** Offline is not a blocker: a request to an absent instance queues until it connects. */
export function dispatchBlocker(instance: InstanceDetail): DispatchBlocker | null {
  if (instance.projects === null || instance.workflows === null) return 'no-read';
  if (!instance.scopes.includes('spend')) return 'no-spend';
  if (instance.projects.length === 0 || instance.workflows.length === 0) return 'no-snapshot';
  return null;
}

/**
 * The Hub projects a workflow graph-only (docs/HUB.md), so no node carries
 * `config`; the canvas treats the key as optional and renders without it.
 */
export function definitionOf(choice: WorkflowChoice): WorkflowDefinitionV2 | null {
  if (choice.graph === null) return null;
  return {
    schema_version: 2,
    id: choice.workflowId,
    name: choice.name,
    nodes: choice.graph.nodes.map(
      (node): NodeConfigV2 => ({ id: node.id, type: node.type, title: node.title, position: node.position }),
    ),
    edges: choice.graph.edges.map((edge) => ({ from: edge.from, to: edge.to })),
  };
}

export function buildDispatchRequest(form: DispatchForm, attachmentIds: string[]): DispatchRequest {
  const agentKind = form.agentKind.trim();
  const model = form.model.trim();
  return {
    project_id: form.projectId,
    workflow_id: form.workflowId,
    title: form.title,
    description: form.description,
    ...(agentKind ? { agent_kind: agentKind } : {}),
    ...(model ? { model } : {}),
    ...(form.effort ? { effort: form.effort } : {}),
    step_overrides: toStepOverrides(form.overrides),
    max_budget_cents: form.maxBudgetCents,
    ...(form.maxWallClockSecs === undefined ? {} : { max_wall_clock_secs: form.maxWallClockSecs }),
    attachment_ids: attachmentIds,
  };
}
