// Unit tests for `src/lib/remoteRunBuckets.ts`.

import { describe, expect, it } from 'vitest';

import { bucketFor, type Bucket } from './remoteRunBuckets';

// Every status the switch names, so a re-spelled case arm goes red here
// rather than silently re-filing runs under a different inbox heading.
const HANDLED: { status: string; bucket: Bucket }[] = [
  { status: 'awaiting_mr', bucket: 'pr_ready' },
  { status: 'completed', bucket: 'pr_ready' },
  { status: 'failed', bucket: 'failed' },
  { status: 'interrupted', bucket: 'failed' },
  { status: 'parked', bucket: 'parked' },
  { status: 'over-budget', bucket: 'parked' },
  { status: 'needs-credentials', bucket: 'needs_credentials' },
  { status: 'unreachable', bucket: 'unreachable' },
  { status: 'cancelled', bucket: 'cancelled' },
  { status: 'pending', bucket: 'running' },
  { status: 'running', bucket: 'running' },
];

describe('bucketFor', () => {
  it.each(HANDLED)("files '$status' under '$bucket'", ({ status, bucket }) => {
    expect(bucketFor(status)).toBe(bucket);
  });

  it('files an unrecognised status under running', () => {
    expect(bucketFor('some_new_runner_state')).toBe('running');
    expect(bucketFor('')).toBe('running');
  });

  it('keeps cancelled out of the failed bucket', () => {
    expect(bucketFor('cancelled')).not.toBe('failed');
  });
});
