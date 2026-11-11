/**
 * Why a step or run failed, as the instance classified it (the protocol's
 * `FailureReason`). A closed list on the wire; a value missing here came from
 * a newer Hub than this page and is shown as received.
 */
const REASON_TEXT: Readonly<Record<string, string>> = {
  usage_limit: 'Provider usage limit reached',
  wall_clock_cap: 'Time limit reached',
  budget_cap: 'Spending cap reached',
  verdict_failed: 'Validation failed',
  environment: 'Environment problem, not the code',
  agent_error: 'The agent reported an error',
  not_retryable: 'Stopped: a retry cannot fix this',
  other: 'Unclassified failure',
};

/** Shown for a failure whose instance predates reasons, or gave none. */
export const NO_REASON_TEXT = 'No reason reported';

/** Event kinds that end in failure, the ones a missing reason is worth saying of. */
const FAILURE_KINDS: ReadonlySet<string> = new Set(['feature_failed', 'step_failed', 'ticket_failed']);

export function failureReasonText(reason: string | null): string {
  if (reason === null) return NO_REASON_TEXT;
  return REASON_TEXT[reason] ?? reason;
}

/**
 * What an event says of its cause: the reason when it gave one, "no reason
 * reported" for a failure that gave none, and nothing for any other event — a
 * cancel only carries a reason when a cap caused it.
 */
export function eventReasonText(kind: string, reason: string | null): string | null {
  if (reason !== null) return failureReasonText(reason);
  return FAILURE_KINDS.has(kind) ? NO_REASON_TEXT : null;
}
