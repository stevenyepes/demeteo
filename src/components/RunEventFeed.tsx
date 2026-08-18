/**
 * `RunEventFeed` — the presentational renderer for the unified append-only
 * run-event log (P1.13). One `RunEvent` becomes one scannable line
 * ("s-impl running · 12k tok · $0.04"), tone-colored by outcome.
 *
 * Presentation only, and deliberately transport-agnostic: both transports emit
 * the same `RunEvent` shape — local rows keyed by feature id (P1.13), remote
 * rows polled from the runner — so `ActivityPanel` hands it either one and gets
 * identical rows. Height and scroll are the caller's.
 */
import React from 'react';
import {
  assignmentEffortLabel,
  parseAssignmentEvidence,
} from '../lib/runEventAssignments';
import type { RunEvent } from '../types';

/** Best-effort human-readable rendering of a `RunEvent.payload_json` — most
 *  kinds carry a plain JSON string (title, branch, url); anything that doesn't
 *  parse as a bare string falls back to the raw text. */
function formatPayload(payloadJson: string | null): string {
  if (!payloadJson) return '';
  try {
    const parsed = JSON.parse(payloadJson);
    return typeof parsed === 'string' ? parsed : JSON.stringify(parsed);
  } catch {
    return payloadJson;
  }
}

const EVENT_KIND_LABEL: Record<string, string> = {
  submitted: 'Submitted',
  project_created: 'Project bootstrapped',
  bootstrapped: 'Repository cloned',
  feature_started: 'Feature started',
  gate_auto_approved: 'Gate auto-approved',
  parked: 'Parked — needs a decision',
  over_budget: 'Over budget',
  needs_credentials: 'Needs credentials',
  cost: 'Total cost',
  terminal_state: 'Reached terminal state',
  pushed: 'Branch pushed',
  pr_opened: 'PR opened',
  pr_open_failed: 'PR failed to open',
  cancelled: 'Cancelled',
  // Per-step telemetry from the engine's DomainEvent stream (P1.13 locally,
  // the runner's RunEventBridge for remote). Structured JSON payloads.
  step_progress: 'Step',
  feature_status: 'Status',
  retry_exhausted: 'Retries exhausted',
  retry_decision: 'Retry',
  env_not_ready: 'Environment not ready',
  gate_required: 'Gate reached',
  gate_decided: 'Gate decided',
  step_output: 'Output',
  agent_spawned: 'Agent spawned',
};

type EventTone = 'default' | 'success' | 'warn' | 'error';

const TONE_CLASS: Record<EventTone, string> = {
  default: 'text-cyan-300',
  success: 'text-emerald-300',
  warn: 'text-amber-300',
  error: 'text-ruby-300',
};

function fmtTokens(t: number | null | undefined): string | null {
  if (t == null) return null;
  if (t >= 1000) return `${(t / 1000).toFixed(t >= 10_000 ? 0 : 1)}k tok`;
  return `${t} tok`;
}

function fmtCost(c: number | null | undefined): string | null {
  if (c == null) return null;
  return `$${c < 0.1 ? c.toFixed(4) : c.toFixed(2)}`;
}

const STEP_STATUS_LABEL: Record<string, string> = {
  running: 'running',
  completed: 'done',
  failed: 'failed',
  pending: 'queued',
  skipped: 'skipped',
  awaiting_gate: 'gate',
};

/**
 * The union of every field the structured event kinds carry. Declared as one
 * all-optional record rather than a discriminated union because the runner is
 * versioned separately from the laptop: a payload from a newer runner may
 * carry fields this build has never heard of, and every read below is already
 * written to survive a missing one.
 */
interface RunEventPayload {
  status?: string;
  step_id?: string;
  tokens?: number;
  cost_usd?: number;
  wall_clock_secs?: number;
  text?: string;
  error_class?: string;
  rule_id?: string;
  action?: string;
  attempt?: number;
  max?: number;
  reason?: string;
  decision?: string;
}

/**
 * Turn one `RunEvent` into a `{ label, detail, tone }` triple for a feed row.
 * Structured per-step kinds get a compact rendering; legacy string-payload
 * kinds fall back to `formatPayload`.
 */
export function describeEvent(
  kind: string,
  payloadJson: string | null,
): { label: string; detail: string; tone: EventTone } {
  const fallbackLabel = EVENT_KIND_LABEL[kind] ?? kind;
  let p: RunEventPayload | null = null;
  if (payloadJson) {
    try {
      p = JSON.parse(payloadJson) as RunEventPayload;
    } catch {
      /* not JSON — handled by formatPayload fallback below */
    }
  }

  switch (kind) {
    case 'agent_spawned': {
      const evidence = parseAssignmentEvidence(kind, payloadJson);
      if (!evidence) break;
      return {
        label: fallbackLabel,
        detail: `Agent ${evidence.agentKind} · Effective effort ${assignmentEffortLabel(evidence.effort)}`,
        tone: 'default',
      };
    }
    case 'step_progress': {
      if (!p || typeof p !== 'object') break;
      const status = String(p.status ?? '');
      const bits = [
        fmtTokens(p.tokens),
        fmtCost(p.cost_usd),
        p.wall_clock_secs != null ? `${p.wall_clock_secs}s` : null,
      ].filter(Boolean);
      const tone: EventTone =
        status === 'failed' ? 'error' : status === 'completed' ? 'success' : 'default';
      const detail = [`${p.step_id} ${STEP_STATUS_LABEL[status] ?? status}`, ...bits].join(' · ');
      return { label: 'Step', detail, tone };
    }
    case 'feature_status':
      return { label: 'Status', detail: String(p?.status ?? ''), tone: 'default' };
    case 'step_output': {
      // Coalesced agent stream text; collapse whitespace to one scannable line.
      const text = String(p?.text ?? '').replace(/\s+/g, ' ').trim();
      return { label: '›', detail: text, tone: 'default' };
    }
    case 'retry_decision':
      return {
        label: fallbackLabel,
        detail: `${p?.step_id ?? 'step'} — ${p?.error_class ?? '?'} → ${p?.rule_id ?? p?.action ?? '?'}${
          p?.attempt != null ? ` (attempt ${p.attempt}${p?.max != null ? `/${p.max}` : ''})` : ''
        }`,
        tone: p?.action === 'fail' ? 'error' : 'warn',
      };
    case 'retry_exhausted':
      return {
        label: fallbackLabel,
        detail: `${p?.step_id ?? 'step'} — attempt ${p?.attempt ?? '?'} of ${p?.max ?? '?'}${
          p?.reason ? `: ${p.reason}` : ''
        }`,
        tone: 'error',
      };
    case 'env_not_ready':
      return {
        label: fallbackLabel,
        detail: `${p?.step_id ?? 'step'} — ${p?.reason ?? ''}`,
        tone: 'error',
      };
    case 'gate_decided':
      return { label: fallbackLabel, detail: String(p?.decision ?? ''), tone: 'default' };
    case 'gate_required':
      return { label: fallbackLabel, detail: '', tone: 'warn' };
    case 'parked':
    case 'over_budget':
    case 'needs_credentials':
      return { label: fallbackLabel, detail: formatPayload(payloadJson), tone: 'warn' };
    case 'pr_open_failed':
    case 'failed':
      return { label: fallbackLabel, detail: formatPayload(payloadJson), tone: 'error' };
    case 'pr_opened':
      return { label: fallbackLabel, detail: formatPayload(payloadJson), tone: 'success' };
  }

  return { label: fallbackLabel, detail: formatPayload(payloadJson), tone: 'default' };
}

/** The event rows themselves — a `<div>` list of timestamp · label · detail.
 *  `emptyHint` shows when there are no events. Scroll/height is the caller's. */
export const RunEventFeed: React.FC<{
  events: RunEvent[];
  emptyHint?: string;
}> = ({ events, emptyHint = 'Waiting for events…' }) => {
  if (events.length === 0) {
    return <p className="text-slate-500">{emptyHint}</p>;
  }
  return (
    <>
      {events.map((e) => {
        const { label, detail, tone } = describeEvent(e.kind, e.payload_json);
        return (
          <div key={e.offset} className="flex items-start gap-2">
            <span className="shrink-0 text-slate-600">
              {new Date(e.created_at).toLocaleTimeString()}
            </span>
            <div className="min-w-0">
              <span className={TONE_CLASS[tone]}>{label}</span>
              {detail && <span className="ml-1.5 break-words text-slate-400">{detail}</span>}
            </div>
          </div>
        );
      })}
    </>
  );
};
