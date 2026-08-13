/**
 * The Runs inbox taxonomy (docs/REMOTE_EXECUTION.md M6.2, design §8):
 * every mirrored remote run collapses into one row of the design doc's
 * table — PR ready / Failed / Parked / Needs credentials / Running /
 * Unreachable. `cancelled` isn't in that table (it's a deliberate user
 * action, not an outcome to chase) so it gets its own low-priority
 * bucket rather than being crowbarred into "Failed".
 *
 * An unrecognised status buckets as `running` rather than throwing: the
 * runner's vocabulary can grow ahead of the desktop, and a status the
 * mirror has never seen is still a run in flight until it reports an
 * outcome.
 */

export type Bucket =
  | 'pr_ready'
  | 'failed'
  | 'parked'
  | 'needs_credentials'
  | 'running'
  | 'unreachable'
  | 'cancelled';

export function bucketFor(status: string): Bucket {
  switch (status) {
    case 'awaiting_mr':
    case 'completed':
      return 'pr_ready';
    case 'failed':
    case 'interrupted':
      return 'failed';
    case 'parked':
    case 'over-budget':
      return 'parked';
    case 'needs-credentials':
      return 'needs_credentials';
    case 'unreachable':
      return 'unreachable';
    case 'cancelled':
      return 'cancelled';
    case 'pending':
    case 'running':
    default:
      return 'running';
  }
}
