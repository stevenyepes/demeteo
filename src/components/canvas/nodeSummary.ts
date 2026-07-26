/**
 * "Config essence" for a node card (task P3.2, PRD §6.3's anti-"identical
 * boxes" rule): the handful of settings that make a graph scannable without
 * opening a panel — which agent/model/effort a node pins, how much it is
 * allowed to write, whether a verifier guards it, and where a failure sends
 * the run.
 *
 * Unlike `schemaForm.ts`, this *is* a best-effort read of conventional config
 * keys (`agent_kind`, `model`, `effort`, `capability`, `verifier`) rather than
 * a schema derivation — a badge is a curatorial choice about what deserves
 * space on a 240px card, and no JSON Schema can express that. The contract is
 * that it degrades silently: a node type using none of these keys simply gets
 * no badges, exactly as it does today, and is never broken by their absence.
 */
import { retrySummary } from './schemaForm';
import type { NodeConfigV2 } from './types';

export type EssenceKind = 'agent' | 'model' | 'effort' | 'capability' | 'flag';

export interface EssenceBadge {
  kind: EssenceKind;
  label: string;
  /** Long form for the card's `title` tooltip. */
  hint: string;
}

export interface NodeEssence {
  badges: EssenceBadge[];
  /** A verifier turn guards this node — rendered as a dot, not a badge. */
  verifier: boolean;
  /** `verdict→implement ×3`, at most two entries on the card. */
  retry: string[];
}

function str(config: Record<string, unknown> | undefined, key: string): string | null {
  const v = config?.[key];
  return typeof v === 'string' && v.trim() ? v.trim() : null;
}

const CAPABILITY_LABELS: Record<string, string> = {
  read_only: 'read-only',
  artifacts: 'artifacts',
  verify: 'verify',
  implement: 'implement',
};

export function nodeEssence(node: NodeConfigV2): NodeEssence {
  const config = node.config;
  const badges: EssenceBadge[] = [];

  const agent = str(config, 'agent_kind');
  if (agent) badges.push({ kind: 'agent', label: agent, hint: `Agent: ${agent}` });

  const model = str(config, 'model');
  if (model) badges.push({ kind: 'model', label: model, hint: `Model: ${model}` });

  const effort = str(config, 'effort');
  if (effort) badges.push({ kind: 'effort', label: effort, hint: `Effort: ${effort}` });

  const capability = str(config, 'capability');
  if (capability) {
    badges.push({
      kind: 'capability',
      label: CAPABILITY_LABELS[capability] ?? capability,
      hint: `Write scope: ${CAPABILITY_LABELS[capability] ?? capability}`,
    });
  }

  if (config?.allow_network === true) {
    badges.push({ kind: 'flag', label: 'net', hint: 'Web search / fetch allowed' });
  }
  if (config?.allow_shell === true) {
    badges.push({ kind: 'flag', label: 'shell', hint: 'Shell allowed' });
  }

  return {
    badges,
    verifier: Boolean(config?.verifier),
    retry: retrySummary(node.retry),
  };
}

/** True when there is nothing to draw — lets the card skip the whole row
 *  instead of rendering an empty strip. */
export function isEssenceEmpty(essence: NodeEssence): boolean {
  return essence.badges.length === 0 && !essence.verifier && essence.retry.length === 0;
}
