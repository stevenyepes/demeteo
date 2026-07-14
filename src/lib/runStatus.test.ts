// Unit tests for `src/lib/runStatus.ts`.
//
// Runner: `tsc --noEmit`. Assertions throw on failure so the type-check
// gate (the project's de-facto test runner for module-level checks)
// surfaces regressions.

import {
  bucketFor,
  featureRunStatus,
  runStatusMeta,
  TERMINAL_STATUSES,
  TONE_CHIP,
  type Bucket,
  type FeatureRunStatusFields,
} from './runStatus';

// ── (1) Every status a Feature row can hold gets its own label ──────────
//
// The bug this guards: ProjectHome used to badge every non-`gated`
// feature "RUNNING FLEET", so a failed or completed run looked live.

const FEATURE_STATUSES: { status: string; label: string; active: boolean }[] = [
  { status: 'pending', label: 'Queued', active: true },
  { status: 'running', label: 'Running', active: true },
  { status: 'awaiting_gate', label: 'Gate needs you', active: false },
  { status: 'awaiting_mr', label: 'PR ready', active: false },
  { status: 'completed', label: 'Completed', active: false },
  { status: 'failed', label: 'Failed', active: false },
  { status: 'cancelled', label: 'Cancelled', active: false },
  { status: 'published', label: 'Published', active: false },
];

for (const { status, label, active } of FEATURE_STATUSES) {
  const meta = runStatusMeta(featureRunStatus({ status }));
  if (meta.label !== label) {
    throw new Error(
      `runStatus: '${status}' should be labelled '${label}', got '${meta.label}'`,
    );
  }
  if (meta.active !== active) {
    throw new Error(
      `runStatus: '${status}' should have active=${active}, got ${meta.active}`,
    );
  }
  if (!TONE_CHIP[meta.tone]) {
    throw new Error(`runStatus: '${status}' has no chip classes for tone '${meta.tone}'`);
  }
}

// Only in-motion statuses earn the pulsing affordance.
const stillPulsing = FEATURE_STATUSES.filter(
  (s) => runStatusMeta(s.status).active && s.status !== 'pending' && s.status !== 'running',
);
if (stillPulsing.length !== 0) {
  throw new Error(
    `runStatus: terminal/blocked statuses must not be active — ${stillPulsing
      .map((s) => s.status)
      .join(', ')}`,
  );
}

// ── (2) Published beats completed ───────────────────────────────────────
//
// MrPublisher sets `status = 'completed'` and the MR fields in one write,
// so `status` alone cannot distinguish a shipped run from a bare one.

for (const mrState of ['draft', 'open', 'merged']) {
  const feature: FeatureRunStatusFields = {
    status: 'completed',
    mr_url: 'https://github.com/acme/repo/pull/7',
    mr_state: mrState,
  };
  if (featureRunStatus(feature) !== 'published') {
    throw new Error(`runStatus: mr_state='${mrState}' with an mr_url should resolve to 'published'`);
  }
  if (runStatusMeta(featureRunStatus(feature)).label !== 'Published') {
    throw new Error(`runStatus: mr_state='${mrState}' should be labelled 'Published'`);
  }
}

// A closed-without-merge PR published nothing → fall back to the row's status.
if (
  featureRunStatus({
    status: 'completed',
    mr_url: 'https://github.com/acme/repo/pull/7',
    mr_state: 'closed',
  }) !== 'completed'
) {
  throw new Error("runStatus: mr_state='closed' should fall through to the feature status");
}

// `mr_state` without an `mr_url`, and vice versa, are not published either.
if (featureRunStatus({ status: 'completed', mr_state: 'open' }) !== 'completed') {
  throw new Error('runStatus: mr_state without an mr_url should not resolve to published');
}
if (featureRunStatus({ status: 'running', mr_url: '', mr_state: 'none' }) !== 'running') {
  throw new Error('runStatus: a run with no MR should keep its own status');
}

// ── (3) Unknown statuses degrade, they do not throw ─────────────────────

const unknown = runStatusMeta(featureRunStatus({ status: 'some_new_state' }));
if (unknown.label !== 'some new state' || unknown.tone !== 'slate' || unknown.active) {
  throw new Error(
    `runStatus: unknown status should fall back to an inert slate chip, got ${JSON.stringify(unknown)}`,
  );
}

// ── (4) Buckets — the coarse triage grouping over the status vocabulary ─
//
// Every status a mirrored run can hold lands in exactly one bucket, and an
// unrecognised one lands in `running`: a status this build predates is far
// likelier than a broken run, so we tell the human "still in motion" rather
// than crying failure.

const BUCKETS: { status: string; bucket: Bucket }[] = [
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
  { status: 'some_new_state', bucket: 'running' },
];

for (const { status, bucket } of BUCKETS) {
  const got = bucketFor(status);
  if (got !== bucket) {
    throw new Error(`runStatus: '${status}' should bucket to '${bucket}', got '${got}'`);
  }
}

// Every terminal status buckets to something inert — none of them may be
// reported as still running, which is what gates the Cancel affordance.
for (const status of TERMINAL_STATUSES) {
  if (bucketFor(status) === 'running') {
    throw new Error(`runStatus: terminal status '${status}' must not bucket to 'running'`);
  }
}

export {};
