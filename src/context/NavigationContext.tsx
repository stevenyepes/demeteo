import { createContext, useCallback, useContext, useMemo, useReducer } from 'react';
import type { AppView } from '../types';

export type NavigationMode = 'push' | 'replace';

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

function shallowEqualView(a: AppView, b: AppView): boolean {
  if (a.kind !== b.kind) return false;
  switch (a.kind) {
    case 'empty-state':
    case 'home':
    case 'new-project':
    case 'create-project':
    case 'project-settings':
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
        && a.gateStepExecutionId === (b as Extract<AppView, { kind: 'detail' }>).gateStepExecutionId;
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
}

const NavigationContext = createContext<NavigationContextValue | null>(null);

const initialState: NavigationState = {
  current: { kind: 'empty-state' },
  backStack: [],
  forwardStack: [],
};

export function NavigationProvider({ children }: { children: React.ReactNode }) {
  const [state, dispatch] = useReducer(navigationReducer, initialState);

  const navigate = useCallback(
    (view: AppView, mode?: NavigationMode) => dispatch({ type: 'NAVIGATE', view, mode }),
    [],
  );
  const goBack = useCallback(() => dispatch({ type: 'BACK' }), []);
  const goForward = useCallback(() => dispatch({ type: 'FORWARD' }), []);

  const value = useMemo<NavigationContextValue>(
    () => ({
      view: state.current,
      canGoBack: state.backStack.length > 0,
      canGoForward: state.forwardStack.length > 0,
      navigate,
      goBack,
      goForward,
    }),
    [state.current, state.backStack.length, state.forwardStack.length, navigate, goBack, goForward],
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