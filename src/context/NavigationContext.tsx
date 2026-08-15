import { createContext, useCallback, useContext, useMemo, useReducer, useRef } from 'react';
import type { AppView } from '../types';

export type NavigationMode = 'push' | 'replace';

/**
 * A navigation someone asked for, in a form that can be held and replayed
 * (task P3.3). Every route change in the app funnels through one of these
 * three, which is what lets a single guard cover the Back arrow, `Escape`,
 * `Cmd+W`, and the mouse back button at once — audit F38 is precisely the bug
 * of guarding one path and forgetting the rest.
 */
export type NavigationIntent =
  | { kind: 'navigate'; view: AppView; mode?: NavigationMode }
  | { kind: 'back' }
  | { kind: 'forward' };

/**
 * Return `false` to block an intent. A blocking guard **owns the follow-up**:
 * it must prompt the user and then either drop the intent or replay it via
 * `proceed`, or the app is stuck.
 */
export type NavigationGuard = (intent: NavigationIntent) => boolean;

export interface NavigationState {
  current: AppView;
  backStack: AppView[];
  forwardStack: AppView[];
}

export const MAX_BACK_STACK = 50;

export type Action =
  | { type: 'NAVIGATE'; view: AppView; mode?: NavigationMode }
  | { type: 'BACK' }
  | { type: 'FORWARD' }
  | { type: 'DROP_INVALID'; replacement: AppView };

/**
 * Whether two views are the same destination, which is what the push path uses
 * to collapse a re-navigation to where the user already is.
 *
 * Every field of an arm has to be listed, and a missing one fails silently in
 * the worse direction: two views that differ only in the forgotten field
 * compare equal, so the push collapses and the navigation never happens. There
 * is no test that catches an omission generically — a new field on `AppView`
 * needs a case here and a case in the suite.
 */
function shallowEqualView(a: AppView, b: AppView): boolean {
  if (a.kind !== b.kind) return false;
  switch (a.kind) {
    case 'empty-state':
    case 'home':
    case 'new-project':
    case 'create-project':
    case 'project-settings':
    case 'code-review':
    case 'workflows':
    case 'providers':
    case 'settings':
    case 'terminals':
    case 'remote-inbox':
      return true;
    case 'detail':
      return b.kind === 'detail'
        && a.featureId === (b as Extract<AppView, { kind: 'detail' }>).featureId
        && a.featureTitle === (b as Extract<AppView, { kind: 'detail' }>).featureTitle
        && a.gateStepExecutionId === (b as Extract<AppView, { kind: 'detail' }>).gateStepExecutionId
        && a.selectedStepId === (b as Extract<AppView, { kind: 'detail' }>).selectedStepId;
    case 'editor':
      return b.kind === 'editor'
        && a.featureId === (b as Extract<AppView, { kind: 'editor' }>).featureId
        && a.featureTitle === (b as Extract<AppView, { kind: 'editor' }>).featureTitle
        && a.editorContext === (b as Extract<AppView, { kind: 'editor' }>).editorContext;
    case 'workflow-editor':
      return b.kind === 'workflow-editor'
        && a.workflowId === (b as Extract<AppView, { kind: 'workflow-editor' }>).workflowId;
    default:
      return false;
  }
}

export function navigationReducer(state: NavigationState, action: Action): NavigationState {
  switch (action.type) {
    case 'NAVIGATE': {
      const mode: NavigationMode = action.mode ?? 'push';
      if (mode === 'replace') {
        if (shallowEqualView(state.current, action.view)) return state;
        return { ...state, current: action.view };
      }
      if (shallowEqualView(state.current, action.view)) return state;
      const nextBack = [...state.backStack, state.current];
      const trimmedBack = nextBack.length > MAX_BACK_STACK
        ? nextBack.slice(nextBack.length - MAX_BACK_STACK)
        : nextBack;
      return {
        current: action.view,
        backStack: trimmedBack,
        forwardStack: [],
      };
    }
    case 'BACK': {
      if (state.backStack.length === 0) return state;
      const previous = state.backStack[state.backStack.length - 1];
      return {
        current: previous,
        backStack: state.backStack.slice(0, -1),
        forwardStack: [...state.forwardStack, state.current],
      };
    }
    case 'FORWARD': {
      if (state.forwardStack.length === 0) return state;
      const next = state.forwardStack[state.forwardStack.length - 1];
      return {
        current: next,
        backStack: [...state.backStack, state.current],
        forwardStack: state.forwardStack.slice(0, -1),
      };
    }
    case 'DROP_INVALID':
      return { ...state, current: action.replacement };
    default:
      return state;
  }
}

interface NavigationContextValue {
  view: AppView;
  canGoBack: boolean;
  canGoForward: boolean;
  navigate: (view: AppView, mode?: NavigationMode) => void;
  goBack: () => void;
  goForward: () => void;
  /** Install a guard that can veto navigations; returns its unregister.
   *  Prefer the `useNavigationGuard` hook, which handles the lifecycle. */
  registerGuard: (guard: NavigationGuard) => () => void;
  /** Perform an intent without consulting guards — how a guard replays the
   *  navigation it blocked once the user has resolved it. */
  proceed: (intent: NavigationIntent) => void;
}

const NavigationContext = createContext<NavigationContextValue | null>(null);

const initialState: NavigationState = {
  current: { kind: 'empty-state' },
  backStack: [],
  forwardStack: [],
};

export function NavigationProvider({ children }: { children: React.ReactNode }) {
  const [state, dispatch] = useReducer(navigationReducer, initialState);

  // Guards live in a ref, newest last: a screen mounted on top of another gets
  // the first say. Held outside React state on purpose — installing a guard
  // must not re-render the whole app under the provider.
  const guards = useRef<NavigationGuard[]>([]);

  const registerGuard = useCallback((guard: NavigationGuard) => {
    guards.current = [...guards.current, guard];
    return () => {
      guards.current = guards.current.filter((g) => g !== guard);
    };
  }, []);

  /** Ask every guard, innermost first. Any veto stops the intent. */
  const allowed = useCallback((intent: NavigationIntent): boolean => {
    for (let i = guards.current.length - 1; i >= 0; i -= 1) {
      if (!guards.current[i](intent)) return false;
    }
    return true;
  }, []);

  const proceed = useCallback((intent: NavigationIntent) => {
    switch (intent.kind) {
      case 'navigate':
        dispatch({ type: 'NAVIGATE', view: intent.view, mode: intent.mode });
        break;
      case 'back':
        dispatch({ type: 'BACK' });
        break;
      case 'forward':
        dispatch({ type: 'FORWARD' });
        break;
    }
  }, []);

  const navigate = useCallback(
    (view: AppView, mode?: NavigationMode) => {
      const intent: NavigationIntent = { kind: 'navigate', view, mode };
      if (allowed(intent)) proceed(intent);
    },
    [allowed, proceed],
  );
  const goBack = useCallback(() => {
    if (allowed({ kind: 'back' })) proceed({ kind: 'back' });
  }, [allowed, proceed]);
  const goForward = useCallback(() => {
    if (allowed({ kind: 'forward' })) proceed({ kind: 'forward' });
  }, [allowed, proceed]);

  const value = useMemo<NavigationContextValue>(
    () => ({
      view: state.current,
      canGoBack: state.backStack.length > 0,
      canGoForward: state.forwardStack.length > 0,
      navigate,
      goBack,
      goForward,
      registerGuard,
      proceed,
    }),
    [
      state.current,
      state.backStack.length,
      state.forwardStack.length,
      navigate,
      goBack,
      goForward,
      registerGuard,
      proceed,
    ],
  );

  return (
    <NavigationContext.Provider value={value}>
      {children}
    </NavigationContext.Provider>
  );
}

export function useNavigation(): NavigationContextValue {
  const ctx = useContext(NavigationContext);
  if (!ctx) throw new Error('useNavigation must be used within NavigationProvider');
  return ctx;
}