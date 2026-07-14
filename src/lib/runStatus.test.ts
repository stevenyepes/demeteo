// Unit tests for `src/lib/runStatus.ts`.

import { describe, expect, it } from 'vitest';

import {
  featureRunStatus,
  runStatusMeta,
  TONE_CHIP,
  type FeatureRunStatusFields,
} from './runStatus';

// The bug this guards: ProjectHome used to badge every non-`gated` feature
// "RUNNING FLEET", so a failed or completed run looked live.
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

describe('featureRunStatus / runStatusMeta', () => {
  it.each(FEATURE_STATUSES)(
    "labels '$status' as '$label' (active=$active)",
    ({ status, label, active }) => {
      const meta = runStatusMeta(featureRunStatus({ status }));

      expect(meta.label).toBe(label);
      expect(meta.active).toBe(active);
      expect(TONE_CHIP[meta.tone]).toBeTruthy();
    },
  );

  it('only lets in-motion statuses claim the pulsing affordance', () => {
    const stillPulsing = FEATURE_STATUSES.filter(
      (s) => runStatusMeta(s.status).active && s.status !== 'pending' && s.status !== 'running',
    );

    expect(stillPulsing.map((s) => s.status)).toEqual([]);
  });
});

// MrPublisher sets `status = 'completed'` and the MR fields in one write, so
// `status` alone cannot distinguish a shipped run from a bare one.
describe('published beats completed', () => {
  it.each(['draft', 'open', 'merged'])("resolves mr_state='%s' to 'published'", (mrState) => {
    const feature: FeatureRunStatusFields = {
      status: 'completed',
      mr_url: 'https://github.com/acme/repo/pull/7',
      mr_state: mrState,
    };

    expect(featureRunStatus(feature)).toBe('published');
    expect(runStatusMeta(featureRunStatus(feature)).label).toBe('Published');
  });

  it('falls through to the feature status when the PR closed without merging', () => {
    expect(
      featureRunStatus({
        status: 'completed',
        mr_url: 'https://github.com/acme/repo/pull/7',
        mr_state: 'closed',
      }),
    ).toBe('completed');
  });

  it('does not treat a half-populated MR as published', () => {
    expect(featureRunStatus({ status: 'completed', mr_state: 'open' })).toBe('completed');
    expect(featureRunStatus({ status: 'running', mr_url: '', mr_state: 'none' })).toBe('running');
  });
});

describe('unknown statuses', () => {
  it('degrades to an inert slate chip instead of throwing', () => {
    const unknown = runStatusMeta(featureRunStatus({ status: 'some_new_state' }));

    expect(unknown).toMatchObject({
      label: 'some new state',
      tone: 'slate',
      active: false,
    });
  });
});
