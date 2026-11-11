import { describe, expect, it } from 'vitest';

import { eventReasonText, failureReasonText, NO_REASON_TEXT } from './failureReason';

describe('failureReasonText', () => {
  it('words every reason the protocol carries', () => {
    expect(failureReasonText('usage_limit')).toBe('Provider usage limit reached');
    expect(failureReasonText('wall_clock_cap')).toBe('Time limit reached');
    expect(failureReasonText('budget_cap')).toBe('Spending cap reached');
    expect(failureReasonText('verdict_failed')).toBe('Validation failed');
    expect(failureReasonText('environment')).toBe('Environment problem, not the code');
    expect(failureReasonText('agent_error')).toBe('The agent reported an error');
    expect(failureReasonText('not_retryable')).toBe('Stopped: a retry cannot fix this');
    expect(failureReasonText('other')).toBe('Unclassified failure');
  });

  it('says no reason was reported when the instance gave none', () => {
    expect(failureReasonText(null)).toBe(NO_REASON_TEXT);
  });

  it('shows a reason from a newer Hub as received', () => {
    expect(failureReasonText('quota_from_the_future')).toBe('quota_from_the_future');
  });
});

describe('eventReasonText', () => {
  it('names a missing reason only on a failure', () => {
    expect(eventReasonText('step_failed', null)).toBe(NO_REASON_TEXT);
    expect(eventReasonText('feature_failed', null)).toBe(NO_REASON_TEXT);
    expect(eventReasonText('feature_cancelled', null)).toBeNull();
    expect(eventReasonText('step_completed', null)).toBeNull();
  });

  it('gives a cancel its cap when a cap caused it', () => {
    expect(eventReasonText('feature_cancelled', 'budget_cap')).toBe('Spending cap reached');
  });

  it('names a missing reason on a ticket failure too', () => {
    expect(eventReasonText('ticket_failed', null)).toBe(NO_REASON_TEXT);
  });
});
