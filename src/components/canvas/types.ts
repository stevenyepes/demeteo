/**
 * TypeScript mirror of the Rust workflow-as-data **schema v2** model
 * (`crates/demeteo-core/src/domain/models/workflow_v2.rs`) plus the
 * per-node-type display vocabulary the canvas renders from.
 *
 * The canvas (task P2.1) consumes *migrated* v2 definitions — the v1 → v2
 * migration (`workflow_migrate.rs`, P1.2) is the only producer today, and the
 * committed fixtures under `__fixtures__/` are emitted straight from it (see
 * the `canvas_fixtures_are_current` regen test) so these types never drift
 * from the wire shape. Fields mirror the serde output: `type` (not
 * `node_type`), snake_case enums, optional blocks omitted when empty.
 */
import {
  Bot,
  ShieldCheck,
  ListChecks,
  GitMerge,
  Flag,
  Terminal,
  Boxes,
  CircleDot,
  type LucideIcon,
} from 'lucide-react';
import type { RunStatusTone } from '../../lib/runStatus';

export interface PositionV2 {
  x: number;
  y: number;
}

export type JoinSemantics = 'all_success' | 'any_success' | 'all_done';
export type RetryStrategy = 'in_place' | 'redirect' | 'fail';

export interface RetryRule {
  strategy: RetryStrategy;
  max_attempts?: number | null;
  backoff_secs?: number | null;
  feedback?: boolean;
  redirect_to?: string | null;
}

export interface RetryPolicy {
  environment?: RetryRule | null;
  verdict?: RetryRule | null;
  agent_failure?: RetryRule | null;
  non_retryable?: RetryRule | null;
}

export interface NodeConfigV2 {
  id: string;
  /** Registry key: `agent | gate | sequence | sync | finalize | command | …`. */
  type: string;
  type_version?: number;
  title: string;
  /** Opaque per-type payload (prompt, agent/model, capability, verifier, …). */
  config?: Record<string, unknown>;
  retry?: RetryPolicy | null;
  join?: JoinSemantics | null;
  /** Co-persisted editor layout; migration synthesizes a vertical column. */
  position?: PositionV2 | null;
}

export interface EdgeConfigV2 {
  from: string;
  to: string;
  /** Sandboxed guard expression (P1.5); absent on unconditional chain edges. */
  when?: string | null;
}

export interface WorkflowDefaults {
  retry?: RetryPolicy | null;
  join?: JoinSemantics | null;
}

export interface WorkflowDefinitionV2 {
  schema_version: number;
  id: string;
  name: string;
  nodes: NodeConfigV2[];
  edges: EdgeConfigV2[];
  defaults?: WorkflowDefaults;
}

/** Display metadata for a node type: card icon, label, and accent tone drawn
 *  from the shared run-status vocabulary (`lib/runStatus.ts`) so the canvas
 *  can't invent a color language of its own. */
export interface NodeTypeMeta {
  label: string;
  icon: LucideIcon;
  tone: RunStatusTone;
}

const NODE_TYPE_META: Record<string, NodeTypeMeta> = {
  agent: { label: 'Agent', icon: Bot, tone: 'cyan' },
  gate: { label: 'Gate', icon: ShieldCheck, tone: 'amber' },
  sequence: { label: 'Sequence', icon: ListChecks, tone: 'violet' },
  sync: { label: 'Sync', icon: GitMerge, tone: 'slate' },
  finalize: { label: 'Finalize', icon: Flag, tone: 'emerald' },
  command: { label: 'Command', icon: Terminal, tone: 'slate' },
  subworkflow: { label: 'Sub-workflow', icon: Boxes, tone: 'violet' },
};

/** Metadata for a node type, with a graceful fallback for unknown/registry
 *  types the palette hasn't taught the canvas about yet. */
export function nodeTypeMeta(type: string): NodeTypeMeta {
  return NODE_TYPE_META[type] ?? { label: type, icon: CircleDot, tone: 'slate' };
}
