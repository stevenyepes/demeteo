import { describe, expect, it } from 'vitest';

import { activityOpensByDefault } from './runMetaDefaults';

const FEATURE_STATUSES = [
  'pending',
  'running',
  'gated',
  'verifying',
  'completed',
  'awaiting_mr',
  'failed',
  'cancelled',
];

describe('activityOpensByDefault', () => {
  it.each(FEATURE_STATUSES)('keeps a local run collapsed when the feature is %s', (featureStatus) => {
    expect(activityOpensByDefault({ remoteStatus: null, featureStatus })).toBe(false);
  });

  it.each(['pending', 'running', 'a-status-this-desktop-has-never-seen'])(
    'opens a detached run whose mirror reads %s behind a running feature',
    (remoteStatus) => {
      expect(activityOpensByDefault({ remoteStatus, featureStatus: 'running' })).toBe(true);
    },
  );

  it.each([
    'completed',
    'awaiting_mr',
    'failed',
    'cancelled',
    'interrupted',
    'parked',
    'over-budget',
    'needs-credentials',
    'unreachable',
  ])('keeps a detached run collapsed when its mirror reads %s', (remoteStatus) => {
    expect(activityOpensByDefault({ remoteStatus, featureStatus: 'running' })).toBe(false);
  });

  it.each(['completed', 'failed', 'cancelled', 'awaiting_mr'])(
    'keeps a stale running mirror collapsed behind a feature that is %s',
    (featureStatus) => {
      expect(activityOpensByDefault({ remoteStatus: 'running', featureStatus })).toBe(false);
    },
  );
});
