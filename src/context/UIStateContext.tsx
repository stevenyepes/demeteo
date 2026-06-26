import { createContext, useContext, useReducer } from 'react';
import type { Provider } from '../types';

interface UIState {
  sidebarCollapsed: boolean;
  commandPaletteOpen: boolean;
  docsPanelOpen: boolean;
  isConnectModalOpen: boolean;
  editingProvider: Provider | null;
  startFeatureOpen: boolean;
  startFeatureWorkflowId: string | null;
}

type UIAction =
  | { type: 'TOGGLE_SIDEBAR' }
  | { type: 'SET_SIDEBAR'; collapsed: boolean }
  | { type: 'SET_COMMAND_PALETTE'; open: boolean }
  | { type: 'SET_DOCS_PANEL'; open: boolean }
  | { type: 'SET_CONNECT_MODAL'; open: boolean; editing?: Provider | null }
  | { type: 'OPEN_START_FEATURE'; workflowId?: string | null }
  | { type: 'CLOSE_START_FEATURE' };

const initial: UIState = {
  sidebarCollapsed: false,
  commandPaletteOpen: false,
  docsPanelOpen: false,
  isConnectModalOpen: false,
  editingProvider: null,
  startFeatureOpen: false,
  startFeatureWorkflowId: null,
};

function reducer(state: UIState, action: UIAction): UIState {
  switch (action.type) {
    case 'TOGGLE_SIDEBAR':
      return { ...state, sidebarCollapsed: !state.sidebarCollapsed };
    case 'SET_SIDEBAR':
      return { ...state, sidebarCollapsed: action.collapsed };
    case 'SET_COMMAND_PALETTE':
      return { ...state, commandPaletteOpen: action.open };
    case 'SET_DOCS_PANEL':
      return { ...state, docsPanelOpen: action.open };
    case 'SET_CONNECT_MODAL':
      return {
        ...state,
        isConnectModalOpen: action.open,
        editingProvider: action.editing !== undefined ? action.editing ?? null : state.editingProvider,
      };
    case 'OPEN_START_FEATURE':
      return { ...state, startFeatureOpen: true, startFeatureWorkflowId: action.workflowId ?? null };
    case 'CLOSE_START_FEATURE':
      return { ...state, startFeatureOpen: false, startFeatureWorkflowId: null };
    default:
      return state;
  }
}

interface UIStateContextValue {
  ui: UIState;
  uiDispatch: React.Dispatch<UIAction>;
}

const UIStateContext = createContext<UIStateContextValue | null>(null);

export function UIStateProvider({ children }: { children: React.ReactNode }) {
  const [ui, uiDispatch] = useReducer(reducer, initial);
  return (
    <UIStateContext.Provider value={{ ui, uiDispatch }}>
      {children}
    </UIStateContext.Provider>
  );
}

export function useUIState(): UIStateContextValue {
  const ctx = useContext(UIStateContext);
  if (!ctx) throw new Error('useUIState must be used within UIStateProvider');
  return ctx;
}
