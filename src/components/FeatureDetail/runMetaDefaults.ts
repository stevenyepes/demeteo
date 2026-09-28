import { bucketFor } from '../../lib/remoteRunBuckets';
import { TERMINAL_STATUSES } from '../../lib/runStatus';

/**
 * Whether the run's Activity log starts open, before the user has touched it.
 *
 * Collapsed is the default: stacked meta chrome above the graph starves the
 * graph box of height. The one exception is a detached run whose remote side is
 * still bootstrapping or running, behind a feature that has not finished:
 * collapsing `ActivityPanel` stops its remote tail's poll, and that tail is the
 * only source of a detached run's bootstrap phases — shut it by default and
 * the bootstrap stepper silently stops advancing. Nothing else has a stepper
 * to freeze. A local run's events arrive whether or not the panel is open. A
 * parked, over-budget or needs-credentials mirror is waiting on the user, not
 * bootstrapping; an interrupted or unreachable one cannot feed a stepper; and
 * a mirror still reading `running` behind a feature that already finished is
 * stale, not live. An unrecognised mirror status reads as in flight, as it
 * does in the Runs inbox (`bucketFor`).
 */

export interface ActivityDefaultInput {
  /** The detached run's mirrored status, or `null` for a local run. */
  remoteStatus: string | null;
  /** The feature's own run status — what the header badge shows. */
  featureStatus: string;
}

export function activityOpensByDefault({ remoteStatus, featureStatus }: ActivityDefaultInput): boolean {
  return (
    remoteStatus !== null &&
    bucketFor(remoteStatus) === 'running' &&
    !TERMINAL_STATUSES.includes(featureStatus)
  );
}
