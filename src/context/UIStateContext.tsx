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
  /**
   * Every overlay currently on screen, newest last (audit F35).
   *
   * A registry rather than a flag per overlay, because the flag list went
   * stale the moment a modal was added without one — and the symptom was not
   * an error but Escape and mouse-back reaching the *view underneath* an open
   * dialog. Ids only: an overlay already closes itself on Escape, so the
   * global handler needs to know that one is open, not how to shut it.
   */
  overlays: string[];
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
  | { type: 'CLOSE_START_FEATURE' }
  | { type: 'PUSH_OVERLAY'; id: string }
  | { type: 'POP_OVERLAY'; id: string };

const initial: UIState = {
  sidebarCollapsed: false,
  overlays: [],
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
    case 'PUSH_OVERLAY':
      // Ids are unique per mount, so a repeat is a double-register rather than
      // a second overlay; adding it twice would leave one behind on unmount.
      return state.overlays.includes(action.id)
        ? state
        : { ...state, overlays: [...state.overlays, action.id] };
    case 'POP_OVERLAY': {
      const overlays = state.overlays.filter((id) => id !== action.id);
      return overlays.length === state.overlays.length ? state : { ...state, overlays };
    }
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

/**
 * The same context, `null` outside a provider instead of throwing.
 *
 * For the one case where absence is not a mistake: `useOverlay` registers an
 * overlay so that the *global* Escape / mouse-back handlers stand down, and
 * those handlers live under this provider. With no provider there are none of
 * them, so there is nothing to tell and nothing to break — which is exactly the
 * shape of a `Modal` rendered alone in a test. Anything that reads or writes
 * UI state should use `useUIState` and get the error.
 */
export function useOptionalUIState(): UIStateContextValue | null {
  return useContext(UIStateContext);
}
