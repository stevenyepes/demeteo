import { createContext, useCallback, useContext, useReducer } from 'react';
import type { AppView } from '../types';

interface State {
  view: AppView;
}

type Action = { type: 'NAVIGATE'; view: AppView };

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case 'NAVIGATE': return { view: action.view };
    default: return state;
  }
}

interface NavigationContextValue {
  view: AppView;
  navigate: (view: AppView) => void;
}

const NavigationContext = createContext<NavigationContextValue | null>(null);

export function NavigationProvider({ children }: { children: React.ReactNode }) {
  const [state, dispatch] = useReducer(reducer, { view: { kind: 'empty-state' } });
  const navigate = useCallback((view: AppView) => dispatch({ type: 'NAVIGATE', view }), []);
  return (
    <NavigationContext.Provider value={{ view: state.view, navigate }}>
      {children}
    </NavigationContext.Provider>
  );
}

export function useNavigation(): NavigationContextValue {
  const ctx = useContext(NavigationContext);
  if (!ctx) throw new Error('useNavigation must be used within NavigationProvider');
  return ctx;
}
