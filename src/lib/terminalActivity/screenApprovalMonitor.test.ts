import { describe, expect, it } from 'vitest';

import { ScreenApprovalDebouncer } from './screenApprovalMonitor';

describe('ScreenApprovalDebouncer', () => {
  it('starts absent and ignores a single transient match (no flap)', () => {
    const d = new ScreenApprovalDebouncer({ enterFrames: 2, exitFrames: 2 });
    expect(d.state).toBe(false);
    // One-frame match must not commit — this is the anti-flap promise (T3.3).
    expect(d.observe(true)).toBeNull();
    expect(d.state).toBe(false);
  });

  it('commits an assertion once the match persists past the threshold', () => {
    const d = new ScreenApprovalDebouncer({ enterFrames: 2, exitFrames: 2 });
    expect(d.observe(true)).toBeNull();
    expect(d.observe(true)).toBe(true);
    expect(d.state).toBe(true);
  });

  it('a match interrupted before the threshold resets the streak', () => {
    const d = new ScreenApprovalDebouncer({ enterFrames: 3, exitFrames: 2 });
    expect(d.observe(true)).toBeNull(); // 1
    expect(d.observe(true)).toBeNull(); // 2
    expect(d.observe(false)).toBeNull(); // interrupt → streak resets
    expect(d.observe(true)).toBeNull(); // 1 again
    expect(d.observe(true)).toBeNull(); // 2
    expect(d.observe(true)).toBe(true); // 3 → commit
  });

  it('retracts once absence persists past the exit threshold', () => {
    const d = new ScreenApprovalDebouncer({ enterFrames: 1, exitFrames: 2 });
    expect(d.observe(true)).toBe(true);
    expect(d.observe(false)).toBeNull(); // one absent frame — still latched
    expect(d.observe(false)).toBe(false); // second → retract
    expect(d.state).toBe(false);
  });

  it('emits only on transitions, not while steady', () => {
    const d = new ScreenApprovalDebouncer({ enterFrames: 1, exitFrames: 1 });
    expect(d.observe(true)).toBe(true);
    expect(d.observe(true)).toBeNull(); // steady asserted
    expect(d.observe(true)).toBeNull();
    expect(d.observe(false)).toBe(false);
    expect(d.observe(false)).toBeNull(); // steady absent
  });

  it('reset(false) retracts a latched approval without debounce', () => {
    const d = new ScreenApprovalDebouncer({ enterFrames: 1, exitFrames: 5 });
    expect(d.observe(true)).toBe(true);
    expect(d.reset(false)).toBe(false); // teardown — immediate
    expect(d.state).toBe(false);
    // Resetting to the already-committed state is a no-op.
    expect(d.reset(false)).toBeNull();
  });
});
