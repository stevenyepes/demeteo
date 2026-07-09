import { createContext, useContext, useReducer } from 'react';
import type { Provider } from '../types';
import type { LaunchStageEntry } from '../components/AttachmentDropzone';

/**
 * Prefill for the Start Feature modal when it is opened from the
 * inline composer on ProjectHome. The composer captures a title and
 * staged attachments, then hands them off; the modal owns the actual
 * launch (Alternative A — one launch surface).
 */
export interface StartFeatureSeed {
  title?: string;
  attachments?: LaunchStageEntry[];
}

interface UIState {
  sidebarCollapsed: boolean;
  commandPaletteOpen: boolean;
  docsPanelOpen: boolean;
  isConnectModalOpen: boolean;
  editingProvider: Provider | null;
  startFeatureOpen: boolean;
  startFeatureWorkflowId: string | null;
  startFeatureSeed: StartFeatureSeed | null;
}

type UIAction =
  | { type: 'TOGGLE_SIDEBAR' }
  | { type: 'SET_SIDEBAR'; collapsed: boolean }
  | { type: 'SET_COMMAND_PALETTE'; open: boolean }
  | { type: 'SET_DOCS_PANEL'; open: boolean }
  | { type: 'SET_CONNECT_MODAL'; open: boolean; editing?: Provider | null }
  | { type: 'OPEN_START_FEATURE'; workflowId?: string | null; seed?: StartFeatureSeed }
  | { type: 'CLOSE_START_FEATURE' };

const initial: UIState = {
  sidebarCollapsed: false,
  commandPaletteOpen: false,
  docsPanelOpen: false,
  isConnectModalOpen: false,
  editingProvider: null,
  startFeatureOpen: false,
  startFeatureWorkflowId: null,
  startFeatureSeed: null,
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
      return {
        ...state,
        startFeatureOpen: true,
        startFeatureWorkflowId: action.workflowId ?? null,
        startFeatureSeed: action.seed ?? null,
      };
    case 'CLOSE_START_FEATURE':
      return { ...state, startFeatureOpen: false, startFeatureWorkflowId: null, startFeatureSeed: null };
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
