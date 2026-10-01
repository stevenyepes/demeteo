import { createContext, useContext, useReducer } from 'react';
import type { Provider } from '../types';
import type { LaunchStageEntry } from '../components/AttachmentDropzone';

/**
 * Prefill for the Start Feature modal. Two sources: the inline composer on
 * ProjectHome, which has only a title and staged attachments (the modal owns
 * the launch — Alternative A, one launch surface), and a draft kept by
 * `CLOSE_START_FEATURE`.
 *
 * A kept draft is the title, description, attachments and target machine —
 * nothing else. Workflow, per-step overrides and budgets are deliberately not
 * kept; the modal re-derives them on the next open. A seed without a
 * `description` is a composer seed, and the modal fills it from the title.
 */
export interface StartFeatureSeed {
  title?: string;
  description?: string;
  attachments?: LaunchStageEntry[];
  /** Detached target; absent means "run here". */
  machineId?: string;
  /**
   * The project a kept draft was kept in; a composer seed may omit it, since
   * it is always the current project's. `StartFeatureHost` restores a tagged
   * draft only while that project is current, and keeps a mismatched one
   * (unshown) so switching back restores it — until any newer seed or kept
   * draft takes the single slot. The match lives there because this reducer
   * cannot see the current project.
   */
  projectId?: string;
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
  /**
   * Set by `LEAVE_START_FEATURE`; the open modal answers it by closing through
   * its own settings link, with the draft built from its live fields.
   */
  startFeatureLeaveRequested: boolean;
}

type UIAction =
  | { type: 'TOGGLE_SIDEBAR' }
  | { type: 'SET_SIDEBAR'; collapsed: boolean }
  | { type: 'SET_COMMAND_PALETTE'; open: boolean }
  | { type: 'SET_DOCS_PANEL'; open: boolean }
  | { type: 'SET_CONNECT_MODAL'; open: boolean; editing?: Provider | null }
  | { type: 'OPEN_START_FEATURE'; workflowId?: string | null; seed?: StartFeatureSeed }
  /**
   * `keepDraft` is for a close the user made to go somewhere else (the
   * runner notice's settings link, reached directly or through
   * `LEAVE_START_FEATURE`): the draft survives in memory and the next open
   * restores it. Escape, Cancel and a launch omit it. It carries title,
   * description, attachments and target machine only — workflow, overrides
   * and budgets are deliberately not kept.
   */
  | { type: 'CLOSE_START_FEATURE'; keepDraft?: StartFeatureSeed }
  /**
   * Asks an open Start Feature modal to leave for settings, keeping its draft.
   * The refusal toast sends this rather than a `keepDraft` of its own because
   * `useLaunchRun` serves `FeatureDetail` and `CodeReviewView` too: the
   * refused launch's params are not the draft whenever the modal was opened
   * while another surface's submit was in flight. Only the modal holds the
   * live draft, so only the modal may produce it.
   */
  | { type: 'LEAVE_START_FEATURE' }
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
  startFeatureLeaveRequested: false,
};

function seedHasContent(seed: StartFeatureSeed | null | undefined): seed is StartFeatureSeed {
  return (
    !!seed &&
    (!!seed.title || !!seed.description || !!seed.machineId || (seed.attachments?.length ?? 0) > 0)
  );
}

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
        // ProjectHome sends a seed even from an empty composer; that must not
        // wipe a draft kept by `CLOSE_START_FEATURE`.
        startFeatureSeed: seedHasContent(action.seed) ? action.seed : state.startFeatureSeed,
        startFeatureLeaveRequested: false,
      };
    case 'CLOSE_START_FEATURE':
      if (!state.startFeatureOpen) return state;
      return {
        ...state,
        startFeatureOpen: false,
        startFeatureWorkflowId: null,
        startFeatureSeed: seedHasContent(action.keepDraft) ? action.keepDraft : null,
        startFeatureLeaveRequested: false,
      };
    case 'LEAVE_START_FEATURE':
      // Closed, there is no draft to keep and nothing to close.
      if (!state.startFeatureOpen || state.startFeatureLeaveRequested) return state;
      return { ...state, startFeatureLeaveRequested: true };
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
