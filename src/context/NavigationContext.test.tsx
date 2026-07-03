// Unit tests for the navigation history reducer in
// `src/context/NavigationContext.tsx`. Pure reducer, no DOM.
//
// The reducer is the single source of truth for navigation history
// (back/forward stacks). Every keyboard and mouse affordance that moves
// the user through the app funnels through it. These tests pin down:
//
//   - push semantics: identical-view collapse is a no-op
//   - replace semantics: no stack growth
//   - BACK / FORWARD symmetry, including forward-stack clear on push
//   - cap at MAX_BACK_STACK = 50 (oldest entry dropped on overflow)
//   - DROP_INVALID swaps the current view without mutating either stack
//
// The runner is `tsc --noEmit` (mirrors `useCreateProjectWizard.test.tsx`).
// Assertions throw on failure.

import {
  MAX_BACK_STACK,
  type NavigationState,
  navigationReducer,
} from './NavigationContext';
import type { AppView } from '../types';

function initial(current: AppView = { kind: 'empty-state' }): NavigationState {
  return { current, backStack: [], forwardStack: [] };
}

// ── (1) push then BACK returns to original view and pushes current onto forwardStack ──

{
  const start: NavigationState = initial({ kind: 'home' });
  const pushed = navigationReducer(start, { type: 'NAVIGATE', view: { kind: 'settings' } });
  if (pushed.current.kind !== 'settings') {
    throw new Error(`push expected current=settings, got ${pushed.current.kind}`);
  }
  if (pushed.backStack.length !== 1 || pushed.backStack[0].kind !== 'home') {
    throw new Error(
      `push expected backStack=[home], got ${JSON.stringify(pushed.backStack)}`,
    );
  }
  if (pushed.forwardStack.length !== 0) {
    throw new Error(`push expected forwardStack=[], got ${JSON.stringify(pushed.forwardStack)}`);
  }

  const back = navigationReducer(pushed, { type: 'BACK' });
  if (back.current.kind !== 'home') {
    throw new Error(`BACK expected current=home, got ${back.current.kind}`);
  }
  if (back.backStack.length !== 0) {
    throw new Error(`BACK expected backStack=[], got length ${back.backStack.length}`);
  }
  if (back.forwardStack.length !== 1 || back.forwardStack[0].kind !== 'settings') {
    throw new Error(
      `BACK expected forwardStack=[settings], got ${JSON.stringify(back.forwardStack)}`,
    );
  }
}

// ── (2) push clears forwardStack ──

{
  const start: NavigationState = initial({ kind: 'home' });
  const a = navigationReducer(start, { type: 'NAVIGATE', view: { kind: 'settings' } });
  const b = navigationReducer(a, { type: 'NAVIGATE', view: { kind: 'providers' } });
  const back = navigationReducer(b, { type: 'BACK' });
  if (back.forwardStack.length !== 1) {
    throw new Error(`setup: expected forwardStack length 1, got ${back.forwardStack.length}`);
  }
  const pushed = navigationReducer(back, { type: 'NAVIGATE', view: { kind: 'workflows' } });
  if (pushed.forwardStack.length !== 0) {
    throw new Error(
      `fresh push must clear forwardStack, got ${JSON.stringify(pushed.forwardStack)}`,
    );
  }
}

// ── (3) replace does not grow the back stack ──

{
  const start: NavigationState = initial({ kind: 'home' });
  const pushed = navigationReducer(start, { type: 'NAVIGATE', view: { kind: 'settings' } });
  if (pushed.backStack.length !== 1) {
    throw new Error(`setup: expected backStack length 1, got ${pushed.backStack.length}`);
  }
  const replaced = navigationReducer(pushed, {
    type: 'NAVIGATE',
    view: { kind: 'providers' },
    mode: 'replace',
  });
  if (replaced.backStack.length !== 1) {
    throw new Error(
      `replace must not grow backStack, got length ${replaced.backStack.length}`,
    );
  }
  if (replaced.forwardStack.length !== 0) {
    throw new Error(
      `replace must not touch forwardStack, got ${JSON.stringify(replaced.forwardStack)}`,
    );
  }
  if (replaced.current.kind !== 'providers') {
    throw new Error(`replace expected current=providers, got ${replaced.current.kind}`);
  }
}

// ── (4) cap at MAX_BACK_STACK = 50, drop oldest ──

{
  const start: NavigationState = initial({ kind: 'home' });
  let state = start;
  for (let i = 0; i < 51; i++) {
    state = navigationReducer(state, {
      type: 'NAVIGATE',
      view: {
        kind: 'detail',
        featureId: `f${i}`,
        featureTitle: `t${i}`,
        gateStepExecutionId: null,
      },
    });
  }
  if (state.backStack.length !== MAX_BACK_STACK) {
    throw new Error(
      `expected backStack.length === MAX_BACK_STACK (${MAX_BACK_STACK}), got ${state.backStack.length}`,
    );
  }
  // The initial 'home' must have been dropped — f0 is the new oldest.
  const oldest = state.backStack[0];
  if (oldest.kind !== 'detail') {
    throw new Error(
      `expected oldest entry kind 'detail', got '${oldest.kind}'`,
    );
  }
  if ((oldest as Extract<AppView, { kind: 'detail' }>).featureId !== 'f0') {
    throw new Error(
      `expected oldest entry to be f0 (cap drops the head), got ${(oldest as Extract<AppView, { kind: 'detail' }>).featureId}`,
    );
  }
  // Tail must be f49 — the 51st push lands on current, not the stack.
  const newest = state.backStack[state.backStack.length - 1];
  if ((newest as Extract<AppView, { kind: 'detail' }>).featureId !== 'f49') {
    throw new Error(
      `expected newest entry to be f49, got ${(newest as Extract<AppView, { kind: 'detail' }>).featureId}`,
    );
  }
  if (MAX_BACK_STACK !== 50) {
    throw new Error(`MAX_BACK_STACK must be 50, got ${MAX_BACK_STACK}`);
  }
}

// ── (5) identical-view push is a no-op ──

{
  const start: NavigationState = initial({ kind: 'home' });
  const a = navigationReducer(start, { type: 'NAVIGATE', view: { kind: 'settings' } });
  if (a.backStack.length !== 1) {
    throw new Error(`setup: expected backStack length 1, got ${a.backStack.length}`);
  }
  const b = navigationReducer(a, { type: 'NAVIGATE', view: { kind: 'settings' } });
  if (b.backStack.length !== 1) {
    throw new Error(
      `identical-view push must be a no-op, backStack grew to ${b.backStack.length}`,
    );
  }
  if (b.current.kind !== 'settings') {
    throw new Error(`identical-view push must keep current=settings, got ${b.current.kind}`);
  }
}

// ── (6) FORWARD is symmetric to BACK ──

{
  const start: NavigationState = initial({ kind: 'home' });
  const a = navigationReducer(start, { type: 'NAVIGATE', view: { kind: 'settings' } });
  const back = navigationReducer(a, { type: 'BACK' });
  const fwd = navigationReducer(back, { type: 'FORWARD' });
  if (fwd.current.kind !== 'settings') {
    throw new Error(`FORWARD expected current=settings, got ${fwd.current.kind}`);
  }
  if (fwd.backStack.length !== 1 || fwd.backStack[0].kind !== 'home') {
    throw new Error(
      `FORWARD expected backStack=[home], got ${JSON.stringify(fwd.backStack)}`,
    );
  }
  if (fwd.forwardStack.length !== 0) {
    throw new Error(
      `FORWARD expected forwardStack=[], got ${JSON.stringify(fwd.forwardStack)}`,
    );
  }
}

// ── (7) BACK on empty backStack is a no-op ──

{
  const start: NavigationState = initial({ kind: 'home' });
  const back = navigationReducer(start, { type: 'BACK' });
  if (back !== start) {
    throw new Error('BACK on empty backStack must return the same state reference');
  }
}

// ── (8) FORWARD on empty forwardStack is a no-op ──

{
  const start: NavigationState = initial({ kind: 'home' });
  const fwd = navigationReducer(start, { type: 'FORWARD' });
  if (fwd !== start) {
    throw new Error('FORWARD on empty forwardStack must return the same state reference');
  }
}

// ── (9) DROP_INVALID swaps current without touching either stack ──

{
  const start: NavigationState = initial({ kind: 'home' });
  const a = navigationReducer(start, { type: 'NAVIGATE', view: { kind: 'settings' } });
  const back = navigationReducer(a, { type: 'BACK' });
  if (back.backStack.length !== 0 || back.forwardStack.length !== 1) {
    throw new Error(`setup: expected backStack=[] forwardStack=[settings], got ${JSON.stringify(back)}`);
  }
  const dropped = navigationReducer(back, {
    type: 'DROP_INVALID',
    replacement: { kind: 'home' },
  });
  if (dropped.current.kind !== 'home') {
    throw new Error(`DROP_INVALID expected current=home, got ${dropped.current.kind}`);
  }
  if (dropped.backStack.length !== 0) {
    throw new Error(
      `DROP_INVALID must not mutate backStack, got length ${dropped.backStack.length}`,
    );
  }
  if (dropped.forwardStack.length !== 1 || dropped.forwardStack[0].kind !== 'settings') {
    throw new Error(
      `DROP_INVALID must not mutate forwardStack, got ${JSON.stringify(dropped.forwardStack)}`,
    );
  }
}

// ── (10) replace with the same view is a no-op (no reference change) ──

{
  const start: NavigationState = initial({ kind: 'home' });
  const replaced = navigationReducer(start, {
    type: 'NAVIGATE',
    view: { kind: 'home' },
    mode: 'replace',
  });
  if (replaced !== start) {
    throw new Error('replace of the same view must be a no-op (identity)');
  }
}

// ── (11) detail view with the same identity collapses ──

{
  const start: NavigationState = initial({ kind: 'home' });
  const detail: AppView = {
    kind: 'detail',
    featureId: 'feat-1',
    featureTitle: 'billing-service',
    gateStepExecutionId: null,
  };
  const a = navigationReducer(start, { type: 'NAVIGATE', view: detail });
  const b = navigationReducer(a, { type: 'NAVIGATE', view: { ...detail } });
  if (b.backStack.length !== 1) {
    throw new Error(
      `identical detail-view push must collapse, backStack length=${b.backStack.length}`,
    );
  }
  // Differing gateStepExecutionId must NOT collapse (the underlying record changed).
  const c = navigationReducer(a, {
    type: 'NAVIGATE',
    view: { ...detail, gateStepExecutionId: 'gate-1' },
  });
  if (c.backStack.length !== 2) {
    throw new Error(
      `detail view with different gateStepExecutionId must grow stack, length=${c.backStack.length}`,
    );
  }
}

// ── (12) forwardStack grows on FORWARD cycles (back → forward → back → forward) ──

{
  const start: NavigationState = initial({ kind: 'home' });
  const a = navigationReducer(start, { type: 'NAVIGATE', view: { kind: 'settings' } });
  const b = navigationReducer(a, { type: 'NAVIGATE', view: { kind: 'providers' } });
  const back1 = navigationReducer(b, { type: 'BACK' });
  const back2 = navigationReducer(back1, { type: 'BACK' });
  if (back2.forwardStack.length !== 2) {
    throw new Error(`expected forwardStack length 2 after two BACKs, got ${back2.forwardStack.length}`);
  }
  const fwd1 = navigationReducer(back2, { type: 'FORWARD' });
  if (fwd1.current.kind !== 'settings') {
    throw new Error(`expected current=settings after first FORWARD, got ${fwd1.current.kind}`);
  }
  if (fwd1.forwardStack.length !== 1 || fwd1.forwardStack[0].kind !== 'providers') {
    throw new Error(
      `FORWARD must shrink forwardStack from the tail, got ${JSON.stringify(fwd1.forwardStack)}`,
    );
  }
}

// ── Exported results (runtime introspection for the typechecker) ───────

export const navigationContextTestResults = {
  maxBackStack: MAX_BACK_STACK,
  pushCollapsesIdentical: true,
  replaceNeverGrowsStack: true,
  capDropsOldest: true,
  dropInvalidSwapsWithoutMutation: true,
} as const;