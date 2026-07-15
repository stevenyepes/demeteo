// The Rust↔TS effort mirror has no codegen: `EffortLevel` is declared twice,
// once in `crates/demeteo-core/src/domain/models/effort.rs` and once in
// `src/lib/effortLevels.ts`. These assertions are the only thing standing
// between the two and a silent divergence, so they spell the Rust side out
// literally rather than deriving anything from the TS module under test.

import { describe, expect, it } from 'vitest';

import {
  clampForAgent,
  clampToSupported,
  DEFAULT_EFFORT,
  EFFORT_LABELS,
  EFFORT_LEVELS,
  isEffortLevel,
  reconcileEffort,
  supportedEffortsFor,
  type EffortLevel,
} from './effortLevels';

describe('the canonical ladder', () => {
  it('is the five lowercase levels Rust serde emits, in ladder order', () => {
    // `EffortLevel::ALL` — and note `XHigh` renders as "xhigh", not "x-high".
    expect(EFFORT_LEVELS).toEqual(['low', 'medium', 'high', 'xhigh', 'max']);
  });

  it('defaults to high, matching EffortLevel::DEFAULT', () => {
    expect(DEFAULT_EFFORT).toBe('high');
  });

  it('labels every level', () => {
    for (const level of EFFORT_LEVELS) {
      expect(EFFORT_LABELS[level]).toBeTruthy();
    }
    expect(Object.keys(EFFORT_LABELS)).toEqual([...EFFORT_LEVELS]);
  });

  it('accepts only ladder members', () => {
    expect(isEffortLevel('xhigh')).toBe(true);
    expect(isEffortLevel('XHigh')).toBe(false);
    expect(isEffortLevel('ultra')).toBe(false);
    expect(isEffortLevel(undefined)).toBe(false);
  });
});

describe('supportedEffortsFor mirrors EffortLevel::supported_for', () => {
  it('declares the Rust table verbatim', () => {
    expect(supportedEffortsFor('claude-code')).toEqual(['low', 'medium', 'high', 'xhigh', 'max']);
    // codex omits `max`: it only exists on some gpt-5.6-* models.
    expect(supportedEffortsFor('codex')).toEqual(['low', 'medium', 'high', 'xhigh']);
    expect(supportedEffortsFor('opencode')).toEqual(['low', 'medium', 'high', 'xhigh', 'max']);
    // hermes has no per-invocation effort control at all.
    expect(supportedEffortsFor('hermes')).toEqual([]);
  });

  it('assumes the full ladder for an agent this build has never heard of', () => {
    // Guessing wide degrades to a clamp in the adapter; guessing empty would
    // wrongly grey out the control for a perfectly capable new agent.
    expect(supportedEffortsFor('some-future-agent')).toEqual(EFFORT_LEVELS);
  });
});

describe('clampForAgent mirrors EffortLevel::clamp_for', () => {
  const KINDS = ['claude-code', 'codex', 'opencode', 'hermes'];

  it('never returns a level the agent did not declare', () => {
    // The exhaustive cross-product, the same shape as the Rust AC4 test.
    for (const kind of KINDS) {
      for (const level of EFFORT_LEVELS) {
        const clamped = clampForAgent(kind, level);
        if (clamped === null) {
          expect(supportedEffortsFor(kind)).toEqual([]);
        } else {
          expect(supportedEffortsFor(kind)).toContain(clamped);
        }
      }
    }
  });

  it('passes a supported level straight through', () => {
    for (const level of EFFORT_LEVELS) {
      expect(clampForAgent('claude-code', level)).toBe(level);
    }
  });

  it('clamps max down to xhigh on codex — the highest level below it', () => {
    expect(clampForAgent('codex', 'max')).toBe('xhigh');
  });

  it('returns null for hermes at every level', () => {
    for (const level of EFFORT_LEVELS) {
      expect(clampForAgent('hermes', level)).toBeNull();
    }
  });

  it('falls back to the lowest supported level when nothing sits below', () => {
    // Unreachable through the shipped table (every agent that declares
    // anything declares `low`), but it is the rule that makes the Rust
    // `clamp_for` total, so it is pinned against the set-taking form.
    const supported: EffortLevel[] = ['high', 'max'];
    expect(clampToSupported(supported, 'low')).toBe('high');
    expect(clampToSupported(supported, 'xhigh')).toBe('high');
    expect(clampToSupported([], 'high')).toBeNull();
  });
});

describe('reconcileEffort keeps a picker honest across harness changes', () => {
  it('leaves the inherit sentinel untouched', () => {
    expect(reconcileEffort('', supportedEffortsFor('codex'))).toBe('');
    expect(reconcileEffort('', [])).toBe('');
  });

  it('keeps a level the new agent still supports', () => {
    expect(reconcileEffort('high', supportedEffortsFor('codex'))).toBe('high');
  });

  it('clamps a level down to what the new agent accepts', () => {
    // codex tops out at xhigh, so a carried-over `max` becomes `xhigh`
    // (what the backend clamp would run) rather than a stale `max`.
    expect(reconcileEffort('max', supportedEffortsFor('codex'))).toBe('xhigh');
  });

  it('clears to inherit when the new agent has no effort control', () => {
    expect(reconcileEffort('max', supportedEffortsFor('hermes'))).toBe('');
  });
});
