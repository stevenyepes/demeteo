// Pure-logic tests for `src/lib/overlayStack.ts`.
//
// Run with:
//   node --test --experimental-strip-types scripts/overlay-stack.test.ts
//
// These cover the six reducer invariants the spec lists in §5.1 (T1-T6),
// plus four extra checks for the helpers (`generateOverlayId`,
// `replaceOverlay`, `pushOverlay.id` generation, `popMany`).

import { test } from 'node:test';
import { strict as assert } from 'node:assert';

import {
  compareOverlayEntries,
  EMPTY_STACK,
  generateOverlayId,
  hasOverlay,
  overlayStackReducer,
  popMany,
  popOverlay,
  pushOverlay,
  replaceOverlay,
  sortOverlayEntries,
  TIER_RANK,
  topOverlay,
} from '../src/lib/overlayStack.ts';
import type {
  OverlayEntry,
  OverlayPriorityTier,
  PushOptions,
} from '../src/lib/overlayStack.ts';

function makeEntry(overrides: Partial<OverlayEntry> & { tier?: OverlayPriorityTier; createdAt?: number; id?: string }): OverlayEntry {
  const tier = overrides.tier ?? 'modal';
  return {
    id: overrides.id ?? `e-${Math.random().toString(36).slice(2, 9)}`,
    tier,
    priority: overrides.priority ?? 0,
    createdAt: overrides.createdAt ?? Date.now(),
    content: undefined,
    onEscape: undefined,
    dismissOnEscape: true,
    restoreFocus: true,
    ...overrides,
  };
}

function opts(overrides: Omit<PushOptions, 'id' | 'createdAt'> & { id?: string; createdAt?: number; tier?: OverlayPriorityTier }): OverlayEntry {
  return makeEntry({
    id: overrides.id,
    tier: overrides.tier ?? 'modal',
    priority: overrides.priority ?? 0,
    createdAt: overrides.createdAt ?? Date.now(),
  });
}

test('T1 — PUSH orders by tier (gate beats modal beats toast)', () => {
  const a = opts({ id: 'a', tier: 'toast' });
  const b = opts({ id: 'b', tier: 'modal' });
  const c = opts({ id: 'c', tier: 'gate' });
  let s = EMPTY_STACK;
  ({ state: s } = pushOverlay(s, { id: a.id, tier: a.tier }));
  ({ state: s } = pushOverlay(s, { id: b.id, tier: b.tier }));
  ({ state: s } = pushOverlay(s, { id: c.id, tier: c.tier }));
  assert.equal(s.entries[0].id, 'c', 'gate must be first');
  assert.equal(s.entries[s.entries.length - 1].tier, 'toast', 'toast must be last');
  assert.equal(s.entries.length, 3);
});

test('T2 — PUSH within same tier orders by priority desc', () => {
  const a = opts({ id: 'a', tier: 'modal', priority: 10 });
  const b = opts({ id: 'b', tier: 'modal', priority: 50 });
  const c = opts({ id: 'c', tier: 'modal', priority: 30 });
  let s = EMPTY_STACK;
  ({ state: s } = pushOverlay(s, { id: a.id, tier: a.tier, priority: a.priority }));
  ({ state: s } = pushOverlay(s, { id: b.id, tier: b.tier, priority: b.priority }));
  ({ state: s } = pushOverlay(s, { id: c.id, tier: c.tier, priority: c.priority }));
  assert.deepEqual(s.entries.map((e) => e.id), ['b', 'c', 'a']);
});

test('T3 — POP removes by id only, leaves others intact', () => {
  const a = opts({ id: 'a', tier: 'palette' });
  const b = opts({ id: 'b', tier: 'palette' });
  const c = opts({ id: 'c', tier: 'palette' });
  let s = EMPTY_STACK;
  ({ state: s } = pushOverlay(s, { id: a.id, tier: a.tier }));
  ({ state: s } = pushOverlay(s, { id: b.id, tier: b.tier }));
  ({ state: s } = pushOverlay(s, { id: c.id, tier: c.tier }));
  s = popOverlay(s, 'b');
  assert.equal(s.entries.length, 2);
  assert.ok(s.entries.find((e) => e.id === 'a'));
  assert.ok(s.entries.find((e) => e.id === 'c'));
  assert.ok(!s.entries.find((e) => e.id === 'b'));
});

test('T4 — POP of absent id is a no-op', () => {
  const a = opts({ id: 'a', tier: 'modal' });
  let s = EMPTY_STACK;
  ({ state: s } = pushOverlay(s, { id: a.id, tier: a.tier }));
  const before = s;
  const after = popOverlay(s, 'nonexistent');
  assert.equal(after, before, 'reference equality — same stack object');
});

test('T5 — REPLACE updates in place; createdAt is preserved', () => {
  const baseTs = 1_700_000_000_000;
  const a = opts({ id: 'a', tier: 'modal', priority: 5, createdAt: baseTs, onEscape: () => 'old' });
  let s = EMPTY_STACK;
  ({ state: s } = pushOverlay(s, { id: a.id, tier: a.tier, priority: a.priority, createdAt: a.createdAt }));
  s = replaceOverlay(s, 'a', { onEscape: () => 'new' });
  assert.equal(s.entries.length, 1);
  const replaced = s.entries[0];
  assert.equal(replaced.createdAt, baseTs, 'createdAt must be preserved across REPLACE');
  assert.equal(replaced.onEscape?.(), 'new');
});

test('T6 — Tie-breaker is createdAt asc (older is bottom)', () => {
  const older = opts({ id: 'older', tier: 'drawer', priority: 0, createdAt: 100 });
  const newer = opts({ id: 'newer', tier: 'drawer', priority: 0, createdAt: 200 });
  let s = EMPTY_STACK;
  ({ state: s } = pushOverlay(s, { id: older.id, tier: older.tier, priority: older.priority, createdAt: older.createdAt }));
  ({ state: s } = pushOverlay(s, { id: newer.id, tier: newer.tier, priority: newer.priority, createdAt: newer.createdAt }));
  // Both at top (only 2); newer should be index 0, older should be index 1.
  assert.equal(s.entries[0].id, 'newer');
  assert.equal(s.entries[1].id, 'older');
});

test('pushOverlay generates a stable id when none supplied', () => {
  const { entry: a } = pushOverlay(EMPTY_STACK, { tier: 'modal' });
  const { entry: b } = pushOverlay(EMPTY_STACK, { tier: 'modal' });
  assert.notEqual(a.id, b.id);
  assert.match(a.id, /^overlay-[0-9a-z]+-[0-9a-z]+$/);
});

test('generateOverlayId is unique across many calls', () => {
  const ids = new Set<string>();
  for (let i = 0; i < 1000; i++) ids.add(generateOverlayId('test'));
  assert.equal(ids.size, 1000);
});

test('PUSH with a duplicate id is idempotent', () => {
  const a: OverlayEntry = makeEntry({ id: 'dup', tier: 'modal' });
  const s1 = overlayStackReducer(EMPTY_STACK, { type: 'PUSH', entry: a });
  const s2 = overlayStackReducer(s1, { type: 'PUSH', entry: { ...a, priority: 999 } });
  assert.equal(s2.entries.length, 1);
  assert.equal(s2.entries[0].priority, 0, 'first push wins on conflict');
});

test('topOverlay returns undefined on empty stack', () => {
  assert.equal(topOverlay(EMPTY_STACK), undefined);
});

test('topOverlay returns the highest-priority entry', () => {
  let s = EMPTY_STACK;
  ({ state: s } = pushOverlay(s, { id: 'low', tier: 'toast', priority: 0 }));
  ({ state: s } = pushOverlay(s, { id: 'high', tier: 'gate', priority: 0 }));
  assert.equal(topOverlay(s)?.id, 'high');
});

test('hasOverlay is true for members, false otherwise', () => {
  let s = EMPTY_STACK;
  ({ state: s } = pushOverlay(s, { id: 'present', tier: 'modal' }));
  assert.equal(hasOverlay(s, 'present'), true);
  assert.equal(hasOverlay(s, 'missing'), false);
});

test('popMany drops multiple ids in one pass', () => {
  let s = EMPTY_STACK;
  ({ state: s } = pushOverlay(s, { id: 'a', tier: 'modal' }));
  ({ state: s } = pushOverlay(s, { id: 'b', tier: 'modal' }));
  ({ state: s } = pushOverlay(s, { id: 'c', tier: 'modal' }));
  s = popMany(s, ['a', 'c']);
  assert.deepEqual(s.entries.map((e) => e.id), ['b']);
});

test('popMany with empty list is a no-op (reference stable)', () => {
  let s = EMPTY_STACK;
  ({ state: s } = pushOverlay(s, { id: 'a', tier: 'modal' }));
  const before = s;
  const after = popMany(s, []);
  assert.equal(after, before);
});

test('replaceOverlay on missing id is a no-op', () => {
  const before = EMPTY_STACK;
  const after = replaceOverlay(before, 'ghost', { tier: 'gate' });
  assert.equal(after, before);
});

test('TIER_RANK places gate above all others, toast at the bottom', () => {
  assert.ok(TIER_RANK.gate > TIER_RANK.modal);
  assert.ok(TIER_RANK.modal > TIER_RANK.palette);
  assert.ok(TIER_RANK.palette > TIER_RANK.drawer);
  assert.ok(TIER_RANK.drawer > TIER_RANK.toast);
});

test('sortOverlayEntries returns a fresh array (no mutation)', () => {
  const original = [opts({ id: 'b', tier: 'toast' }), opts({ id: 'a', tier: 'gate' })];
  const sorted = sortOverlayEntries(original);
  assert.equal(original[0].id, 'b', 'input order preserved');
  assert.equal(sorted[0].id, 'a', 'sorted output is correct');
});

test('compareOverlayEntries is a stable total order', () => {
  const a = opts({ id: 'a', tier: 'modal', priority: 0, createdAt: 1 });
  const b = opts({ id: 'b', tier: 'modal', priority: 0, createdAt: 2 });
  assert.ok(compareOverlayEntries(b, a) < 0, 'newer sorts higher');
  assert.ok(compareOverlayEntries(a, a) === 0, 'reflexive');
});
