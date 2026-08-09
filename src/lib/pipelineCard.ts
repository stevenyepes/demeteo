/**
 * Everything one pipeline row in `ProjectHome` shows, derived in one place.
 *
 * The card used to spell this inline: two IIFEs re-deriving the workflow and
 * transport badges on every render, next to a status chip whose weight was
 * indistinguishable from the feature id beside it. Both problems have the same
 * cause — a policy decision written inside a render — so the fix is the shape
 * below, not a memo.
 *
 * The three fields mirror the redesign's three-tier read
 * (docs/UI_REDESIGN_PLAN.md §3.3): `scan` identifies the run at a glance,
 * `context` is secondary weight, `detail` is on demand. Grouping them in the
 * type is what stops a detail-tier field from being rendered at scan-tier
 * weight — the drift that produced eight competing elements per row.
 *
 * Tones are {@link RunStatusTone} values, never class strings: the component
 * reads the class out of `TONE_CHIP`/`TONE_ACCENT` so status colour keeps
 * resolving through one registry (ux-audit F27).
 */

import { segmentFor } from './pipelineFilter';
import { featureRunStatus, runStatusMeta, type RunStatusMeta, type RunStatusTone } from './runStatus';
import { formatCost, formatTokens } from './utils';
import { classifyWorkflowBadge, type WorkflowBadge, type WorkflowMeta } from './workflowBadge';
import type { Feature } from '../types';

/** Where the run executes, and the hover text that says so in full. */
export interface TransportBadge {
  label: string;
  tone: RunStatusTone;
  title: string;
}

export interface PipelineCardScan {
  title: string;
  status: RunStatusMeta;
  /** `Feature.duration` is already a formatted wall-clock string from the backend. */
  elapsed: string;
  /**
   * A gate or a credential prompt — the runs §3.2 sorts to the top of the list.
   * Read out of `segmentFor` rather than re-derived from the tone, because the
   * card's affordance and the list's ordering have to answer this the same way:
   * an amber-but-still-moving run (`bootstrapping`) is nobody's decision to
   * make, and a card claiming otherwise about a row the filter left in `active`
   * is the F27 drift in a second vocabulary.
   */
  needsYou: boolean;
}

export interface PipelineCardContext {
  workflow: WorkflowBadge;
  transport: TransportBadge;
  cost: string;
  tokens: string;
}

export interface PipelineCardDetail {
  featureId: string;
  /** `null` when there is nothing to show, so the card renders no empty block. */
  description: string | null;
}

export interface PipelineCardMeta {
  scan: PipelineCardScan;
  context: PipelineCardContext;
  detail: PipelineCardDetail;
}

export interface PipelineCardInput {
  feature: Feature;
  workflowById: ReadonlyMap<string, WorkflowMeta>;
  /** Run is owned by the runner: true regardless of the project's compute type. */
  detached: boolean;
  computeType: string | undefined;
  remoteHost: string | null | undefined;
}

function transportBadge(input: PipelineCardInput): TransportBadge {
  if (input.detached) {
    return {
      label: 'Detached',
      tone: 'cyan',
      title: 'Runs detached under the runner — continues even if this app is closed',
    };
  }

  if (input.computeType?.trim().toLowerCase() === 'remote') {
    const host = input.remoteHost?.trim();
    return {
      label: 'Remote · SSH',
      tone: 'cyan',
      title: `Executes on ${host || 'the project machine'} over SSH`,
    };
  }

  return { label: 'Local', tone: 'slate', title: 'Executes on this machine' };
}

export function pipelineCardMeta(input: PipelineCardInput): PipelineCardMeta {
  const { feature } = input;
  const status = runStatusMeta(featureRunStatus(feature));
  const description = feature.description?.trim();

  return {
    scan: {
      title: feature.title,
      status,
      elapsed: feature.duration,
      needsYou: segmentFor(feature) === 'needs-you',
    },
    context: {
      workflow: classifyWorkflowBadge(feature, input.workflowById),
      transport: transportBadge(input),
      cost: formatCost(feature.total_cost),
      tokens: formatTokens(feature.tokens),
    },
    detail: {
      featureId: feature.id,
      description: description || null,
    },
  };
}
