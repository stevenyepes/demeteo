// Unit tests for the navigation reducer in `src/context/NavigationContext.tsx`.
//
// The reducer is pure, so it is exercised directly — no provider mount needed.

import { describe, expect, it } from 'vitest';

import { MAX_BACK_STACK, type NavigationState, navigationReducer } from './NavigationContext';
import type { AppView } from '../types';

function initial(current: AppView = { kind: 'empty-state' }): NavigationState {
  return { current, backStack: [], forwardStack: [] };
}

const home = initial({ kind: 'home' });

function detailView(featureId: string, gateStepExecutionId: string | null = null): AppView {
  return {
    kind: 'detail',
    featureId,
    featureTitle: `t-${featureId}`,
    gateStepExecutionId,
  };
}

function featureIdOf(view: AppView): string | undefined {
  return view.kind === 'detail' ? view.featureId : undefined;
}

describe('NAVIGATE (push)', () => {
  it('moves the old view onto the back stack', () => {
    const pushed = navigationReducer(home, { type: 'NAVIGATE', view: { kind: 'settings' } });

    expect(pushed.current.kind).toBe('settings');
    expect(pushed.backStack.map((v) => v.kind)).toEqual(['home']);
    expect(pushed.forwardStack).toEqual([]);
  });

  it('clears the forward stack', () => {
    const a = navigationReducer(home, { type: 'NAVIGATE', view: { kind: 'settings' } });
    const b = navigationReducer(a, { type: 'NAVIGATE', view: { kind: 'providers' } });
    const back = navigationReducer(b, { type: 'BACK' });
    expect(back.forwardStack).toHaveLength(1);

    const pushed = navigationReducer(back, { type: 'NAVIGATE', view: { kind: 'workflows' } });

    expect(pushed.forwardStack).toEqual([]);
  });

  it('collapses a push of the view already on screen', () => {
    const a = navigationReducer(home, { type: 'NAVIGATE', view: { kind: 'settings' } });
    const b = navigationReducer(a, { type: 'NAVIGATE', view: { kind: 'settings' } });

    expect(b.backStack).toHaveLength(1);
    expect(b.current.kind).toBe('settings');
  });

  it('collapses an identical detail view but not one with a different gate', () => {
    const detail = detailView('feat-1');
    const a = navigationReducer(home, { type: 'NAVIGATE', view: detail });

    const same = navigationReducer(a, { type: 'NAVIGATE', view: { ...detail } });
    expect(same.backStack).toHaveLength(1);

    // A different gateStepExecutionId means the underlying record changed.
    const differentGate = navigationReducer(a, {
      type: 'NAVIGATE',
      view: detailView('feat-1', 'gate-1'),
    });
    expect(differentGate.backStack).toHaveLength(2);
  });
});

describe('NAVIGATE (replace)', () => {
  it('swaps the current view without growing either stack', () => {
    const pushed = navigationReducer(home, { type: 'NAVIGATE', view: { kind: 'settings' } });

    const replaced = navigationReducer(pushed, {
      type: 'NAVIGATE',
      view: { kind: 'providers' },
      mode: 'replace',
    });

    expect(replaced.current.kind).toBe('providers');
    expect(replaced.backStack).toHaveLength(1);
    expect(replaced.forwardStack).toEqual([]);
  });

  it('is an identity no-op when replacing a view with itself', () => {
    const replaced = navigationReducer(home, {
      type: 'NAVIGATE',
      view: { kind: 'home' },
      mode: 'replace',
    });

    expect(replaced).toBe(home);
  });
});

describe('the back stack cap', () => {
  it('holds at MAX_BACK_STACK and drops the oldest entry', () => {
    expect(MAX_BACK_STACK).toBe(50);

    let state = home;
    for (let i = 0; i < 51; i++) {
      state = navigationReducer(state, { type: 'NAVIGATE', view: detailView(`f${i}`) });
    }

    expect(state.backStack).toHaveLength(MAX_BACK_STACK);
    // The original 'home' fell off the head; f0 is the new oldest. The 51st
    // push landed on `current`, so f49 is the newest entry on the stack.
    expect(featureIdOf(state.backStack[0])).toBe('f0');
    expect(featureIdOf(state.backStack[state.backStack.length - 1])).toBe('f49');
  });
});

describe('BACK / FORWARD', () => {
  it('BACK restores the previous view and banks the current one for forward', () => {
    const pushed = navigationReducer(home, { type: 'NAVIGATE', view: { kind: 'settings' } });

    const back = navigationReducer(pushed, { type: 'BACK' });

    expect(back.current.kind).toBe('home');
    expect(back.backStack).toEqual([]);
    expect(back.forwardStack.map((v) => v.kind)).toEqual(['settings']);
  });

  it('FORWARD is symmetric to BACK', () => {
    const a = navigationReducer(home, { type: 'NAVIGATE', view: { kind: 'settings' } });
    const back = navigationReducer(a, { type: 'BACK' });

    const fwd = navigationReducer(back, { type: 'FORWARD' });

    expect(fwd.current.kind).toBe('settings');
    expect(fwd.backStack.map((v) => v.kind)).toEqual(['home']);
    expect(fwd.forwardStack).toEqual([]);
  });

  it('FORWARD consumes the forward stack from the tail', () => {
    const a = navigationReducer(home, { type: 'NAVIGATE', view: { kind: 'settings' } });
    const b = navigationReducer(a, { type: 'NAVIGATE', view: { kind: 'providers' } });
    const back2 = navigationReducer(navigationReducer(b, { type: 'BACK' }), { type: 'BACK' });
    expect(back2.forwardStack).toHaveLength(2);

    const fwd = navigationReducer(back2, { type: 'FORWARD' });

    expect(fwd.current.kind).toBe('settings');
    expect(fwd.forwardStack.map((v) => v.kind)).toEqual(['providers']);
  });

  // Identity, not just equality — a new object would re-render every consumer.
  it('BACK and FORWARD are identity no-ops on an empty stack', () => {
    expect(navigationReducer(home, { type: 'BACK' })).toBe(home);
    expect(navigationReducer(home, { type: 'FORWARD' })).toBe(home);
  });
});

describe('DROP_INVALID', () => {
  it('swaps the current view without touching either stack', () => {
    const a = navigationReducer(home, { type: 'NAVIGATE', view: { kind: 'settings' } });
    const back = navigationReducer(a, { type: 'BACK' });

    const dropped = navigationReducer(back, {
      type: 'DROP_INVALID',
      replacement: { kind: 'home' },
    });

    expect(dropped.current.kind).toBe('home');
    expect(dropped.backStack).toEqual([]);
    expect(dropped.forwardStack.map((v) => v.kind)).toEqual(['settings']);
  });
});
