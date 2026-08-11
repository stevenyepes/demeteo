import { formatDuration } from '../../../lib/utils';

/** Human label for a failure class (`error_class` / retry-policy key). */
const CLASS_LABELS: Record<string, string> = {
  environment: 'Environment',
  verdict: 'Verdict',
  agent_failure: 'Agent failure',
  non_retryable: 'Non-retryable',
};

export function classLabel(cls: string): string {
  return CLASS_LABELS[cls] ?? cls.replace(/_/g, ' ');
}

/** ms → the shared seconds-based duration formatter. */
export function formatMs(ms: number | null | undefined): string {
  if (ms == null) return '—';
  return formatDuration(ms / 1000);
}

/**
 * Cost for one attempt or task, not for a run.
 *
 * Deliberately not `lib/utils`' `formatCost`, which floors at `<$0.01` and
 * renders a missing number as `$0.00`. At this granularity both lie: a
 * sub-cent attempt is a real, distinguishable number, and "no cost recorded"
 * has to read differently from "cost was zero".
 */
export function formatCost(cost: number | null | undefined): string {
  if (cost == null) return '—';
  return `$${cost.toFixed(cost < 1 ? 4 : 2)}`;
}
