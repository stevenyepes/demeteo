import { describe, expect, it } from 'vitest';

import { describeStaleness, isBehind } from './staleness';
import type { FeatureDrift } from '../types';

function drift(behind: number | null, fetched = true): FeatureDrift {
  return {
    divergence: { behind, ahead: 3 },
    base_ref: 'origin/main',
    fetched,
    checked_at: 1_760_000_000_000,
  };
}

describe('describeStaleness', () => {
  it('counts commits the branch is missing, not the ones it added', () => {
    const chip = describeStaleness(drift(4));
    expect(chip).toMatchObject({ label: '4 behind', tone: 'cyan' });
    expect(chip?.title).toContain('origin/main');
  });

  it('says up to date only for a count that was actually taken', () => {
    expect(describeStaleness(drift(0))).toMatchObject({
      label: 'Up to date',
      tone: 'emerald',
    });
  });

  it('keeps an unmeasurable branch distinct from a synced one', () => {
    const chip = describeStaleness(drift(null));
    expect(chip).toMatchObject({ label: 'Drift unknown', tone: 'slate' });
    expect(chip?.label).not.toContain('0');
  });

  it('says which moment the count belongs to when no fetch was made', () => {
    expect(describeStaleness(drift(2, false))?.title).toContain('last time');
  });

  it('renders nothing at all before a reading lands', () => {
    expect(describeStaleness(null)).toBeNull();
  });
});

describe('isBehind', () => {
  it('does not read an unmeasurable branch as a stale one', () => {
    expect(isBehind({ behind: null, ahead: null })).toBe(false);
    expect(isBehind({ behind: 0, ahead: 2 })).toBe(false);
    expect(isBehind({ behind: 1, ahead: 0 })).toBe(true);
  });
});
