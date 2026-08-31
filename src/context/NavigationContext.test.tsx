// Unit tests for the navigation reducer in `src/context/NavigationContext.tsx`.
//
// The reducer is pure, so it is exercised directly — no provider mount needed.
// The guard registry (task P3.3) is provider state, so its suite at the bottom
// mounts one.

import { useEffect, useState } from 'react';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import {
  MAX_BACK_STACK,
  NavigationProvider,
  navigationReducer,
  useNavigation,
  type NavigationIntent,
  type NavigationState,
} from './NavigationContext';
import type { AppView } from '../types';

function initial(current: AppView = { kind: 'empty-state' }): NavigationState {
  return { current, backStack: [], forwardStack: [] };
}

const home = initial({ kind: 'home' });

type DetailView = Extract<AppView, { kind: 'detail' }>;

function detailView(featureId: string, gateStepExecutionId: string | null = null): DetailView {
  return {
    kind: 'detail',
    featureId,
    featureTitle: `t-${featureId}`,
    gateStepExecutionId,
  };
}

function discoveryView(discoveryId: string, discoveryTitle: string): AppView {
  return { kind: 'discovery', discoveryId, discoveryTitle };
}

function askView(projectId: string): AppView {
  return { kind: 'ask', projectId };
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

  // A no-field variant only collapses if `shallowEqualView` lists it. Omit the
  // case and it falls to the `default: false` arm, which pushes a second copy
  // of the screen the user is already on and buries the real previous view one
  // Back press deeper.
  it('collapses a repeat push of code-review', () => {
    const a = navigationReducer(home, { type: 'NAVIGATE', view: { kind: 'code-review' } });
    const b = navigationReducer(a, { type: 'NAVIGATE', view: { kind: 'code-review' } });

    expect(b).toBe(a);
    expect(b.backStack.map((v) => v.kind)).toEqual(['home']);
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

  // The Discovery arm carries two fields. Drop either from `shallowEqualView`
  // and this goes green on the first half while the push it should have made
  // silently collapses — which on screen is a click on a discovery row that
  // leaves the previous discovery open.
  it('collapses an identical discovery view but not a different one', () => {
    const first = discoveryView('dsc-1', 'Runner serves more than one client');
    const a = navigationReducer(home, { type: 'NAVIGATE', view: first });

    const same = navigationReducer(a, { type: 'NAVIGATE', view: { ...first } });
    expect(same).toBe(a);

    const otherId = navigationReducer(a, {
      type: 'NAVIGATE',
      view: discoveryView('dsc-2', 'Runner serves more than one client'),
    });
    expect(otherId.backStack).toHaveLength(2);

    // A retitled discovery is the same row with a new name: the header renders
    // this title before `discovery_get` answers, so a collapse here would keep
    // showing the old one.
    const otherTitle = navigationReducer(a, {
      type: 'NAVIGATE',
      view: discoveryView('dsc-1', 'What a gate should let you edit'),
    });
    expect(otherTitle.backStack).toHaveLength(2);
  });

  // The Ask arm carries one field, the project id — the thread list lives
  // inside the workspace itself rather than on the view, so there is no
  // second field to drop.
  it('collapses an identical ask view but not a different project', () => {
    const first = askView('proj-1');
    const a = navigationReducer(home, { type: 'NAVIGATE', view: first });

    const same = navigationReducer(a, { type: 'NAVIGATE', view: { ...first } });
    expect(same).toBe(a);

    const otherProject = navigationReducer(a, {
      type: 'NAVIGATE',
      view: askView('proj-2'),
    });
    expect(otherProject.backStack).toHaveLength(2);
  });
});

// ── Step selection as a destination (plan §3.5) ───────────────────────────────
//
// Selection lives on the view so that "which step was I looking at" survives a
// navigation. That only works if the reducer treats it as part of the
// destination: a field `shallowEqualView` does not compare is a field the push
// path collapses away, and the symptom is not an error — it is a click on a
// step that does nothing, with the previous step still in the inspector.

describe('selectedStepId', () => {
  function selecting(stepId: string | null): AppView {
    return { ...detailView('feat-1'), selectedStepId: stepId };
  }

  it('pushes a step selection as its own entry', () => {
    const a = navigationReducer(home, { type: 'NAVIGATE', view: selecting('step-1') });
    const b = navigationReducer(a, { type: 'NAVIGATE', view: selecting('step-2') });

    expect(b.current).toEqual(selecting('step-2'));
    expect(b.backStack).toHaveLength(2);
    expect(b.backStack[1]).toEqual(selecting('step-1'));
  });

  it('collapses a re-selection of the step already showing', () => {
    const a = navigationReducer(home, { type: 'NAVIGATE', view: selecting('step-1') });

    const again = navigationReducer(a, { type: 'NAVIGATE', view: selecting('step-1') });

    expect(again).toBe(a);
  });

  it('distinguishes a selection from the same view with none', () => {
    const a = navigationReducer(home, { type: 'NAVIGATE', view: detailView('feat-1') });

    const selected = navigationReducer(a, { type: 'NAVIGATE', view: selecting('step-1') });
    expect(selected.backStack).toHaveLength(2);

    const cleared = navigationReducer(selected, { type: 'NAVIGATE', view: selecting(null) });
    expect(cleared.backStack).toHaveLength(3);
  });

  it('carries the selection back and forward', () => {
    const a = navigationReducer(home, { type: 'NAVIGATE', view: selecting('step-1') });
    const b = navigationReducer(a, { type: 'NAVIGATE', view: selecting('step-2') });

    const back = navigationReducer(b, { type: 'BACK' });
    expect(back.current).toEqual(selecting('step-1'));

    const fwd = navigationReducer(back, { type: 'FORWARD' });
    expect(fwd.current).toEqual(selecting('step-2'));
  });

  // Selection changes as the user moves down a run, so it is the field most
  // likely to be swapped in place; `replace` keeps that out of the back stack.
  it('swaps in place under replace, leaving the stacks alone', () => {
    const a = navigationReducer(home, { type: 'NAVIGATE', view: selecting('step-1') });

    const replaced = navigationReducer(a, {
      type: 'NAVIGATE',
      view: selecting('step-2'),
      mode: 'replace',
    });

    expect(replaced.current).toEqual(selecting('step-2'));
    expect(replaced.backStack).toHaveLength(1);
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

// ── Navigation guards (task P3.3) ─────────────────────────────────────────────
//
// The guard registry lives on the provider rather than the reducer, so these
// mount it. What matters is that *every* route change is vetoable from one
// place (audit F38 is the bug of guarding one exit and missing three), that
// guards stack innermost-first, and that a blocked intent can be replayed
// verbatim once the screen has resolved it.

describe('navigation guards', () => {
  function Probe({
    guard,
    label = 'go',
  }: {
    guard?: (intent: NavigationIntent) => boolean;
    label?: string;
  }) {
    const { view, navigate, goBack, registerGuard, proceed } = useNavigation();
    const [held, setHeld] = useState<NavigationIntent | null>(null);

    useEffect(() => {
      if (!guard) return;
      return registerGuard((intent) => {
        setHeld(intent);
        return guard(intent);
      });
    }, [guard, registerGuard]);

    return (
      <div>
        <span data-testid="view">{view.kind}</span>
        <button type="button" onClick={() => navigate({ kind: 'settings' })}>
          {label}
        </button>
        <button type="button" onClick={goBack}>
          back
        </button>
        <button type="button" onClick={() => held && proceed(held)}>
          replay
        </button>
      </div>
    );
  }

  afterEach(cleanup);

  it('lets navigation through when no guard is installed', () => {
    render(
      <NavigationProvider>
        <Probe />
      </NavigationProvider>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'go' }));
    expect(screen.getByTestId('view')).toHaveTextContent('settings');
  });

  it('blocks navigate and goBack alike, then replays the held intent', () => {
    render(
      <NavigationProvider>
        <Probe guard={() => false} />
      </NavigationProvider>,
    );

    fireEvent.click(screen.getByRole('button', { name: 'go' }));
    expect(screen.getByTestId('view')).toHaveTextContent('empty-state');

    // The same veto covers the back stack — no second opt-in needed.
    fireEvent.click(screen.getByRole('button', { name: 'back' }));
    expect(screen.getByTestId('view')).toHaveTextContent('empty-state');

    // `proceed` is the escape hatch a guard owns: replay what it blocked.
    // (Ask again first — the last blocked intent was the `back`, and going
    // back from the initial view is a no-op by design.)
    fireEvent.click(screen.getByRole('button', { name: 'go' }));
    fireEvent.click(screen.getByRole('button', { name: 'replay' }));
    expect(screen.getByTestId('view')).toHaveTextContent('settings');
  });

  it('stops guarding once the screen unmounts', () => {
    const { unmount } = render(
      <NavigationProvider>
        <Probe guard={() => false} />
        <Probe label="outer" />
      </NavigationProvider>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'outer' }));
    expect(screen.getAllByTestId('view')[0]).toHaveTextContent('empty-state');
    unmount();

    // A fresh tree with no guard navigates freely — the old guard did not
    // outlive its component.
    render(
      <NavigationProvider>
        <Probe />
      </NavigationProvider>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'go' }));
    expect(screen.getByTestId('view')).toHaveTextContent('settings');
  });
});
