import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
} from 'react';
import type { ReactNode } from 'react';

import type {
  SessionInfo,
  TerminalPanelState,
  TerminalTabDescriptor,
} from '../types';
import {
  closeTerminalSession,
  listTerminalSessions,
  renameTerminalSession,
  resolveRepoDir,
  startTerminalSession,
} from '../lib/terminal';
import { useTauriEvent } from '../hooks/useTauriEvent';

type TerminalPanelAction =
  | { type: 'OPEN_TAB'; tab: TerminalTabDescriptor }
  | { type: 'TAB_SESSION_ATTACHED'; tabId: string; sessionId: string }
  | { type: 'TAB_SESSION_FAILED'; tabId: string }
  | { type: 'CLOSE_TAB'; tabId: string }
  | { type: 'FOCUS_TAB'; tabId: string }
  | { type: 'SET_TITLE'; tabId: string; title: string }
  | { type: 'TOGGLE_PANEL' }
  | { type: 'TAB_ENDED'; sessionId: string }
  | { type: 'STARTUP_RECONCILE'; tabs: TerminalTabDescriptor[] };

const initialState: TerminalPanelState = {
  tabs: [],
  activeTabId: null,
  collapsed: false,
};

function defaultTabTitle(
  input: { machineLabel: string; repoPath?: string },
  index: number,
): string {
  if (input.repoPath) {
    const parts = input.repoPath.split('/').filter(Boolean);
    const last = parts[parts.length - 1];
    if (last) return last;
  }
  if (input.machineLabel) return input.machineLabel;
  return `terminal ${index + 1}`;
}

function isSameLogicalTab(a: TerminalTabDescriptor, b: TerminalTabDescriptor): boolean {
  return (
    a.machineId === b.machineId &&
    (a.repoPath ?? null) === (b.repoPath ?? null) &&
    (a.workBranch ?? null) === (b.workBranch ?? null)
  );
}

function reducer(
  state: TerminalPanelState,
  action: TerminalPanelAction,
): TerminalPanelState {
  switch (action.type) {
    case 'OPEN_TAB': {
      if (state.tabs.some((t) => t.tabId === action.tab.tabId)) return state;
      const existing = state.tabs.find((t) => isSameLogicalTab(t, action.tab));
      if (existing) {
        return { ...state, activeTabId: existing.tabId, collapsed: false };
      }
      const tab: TerminalTabDescriptor = {
        ...action.tab,
        title:
          action.tab.title ||
          defaultTabTitle(action.tab, state.tabs.length),
      };
      return {
        ...state,
        tabs: [...state.tabs, tab],
        activeTabId: tab.tabId,
        collapsed: false,
      };
    }
    case 'TAB_SESSION_ATTACHED':
      return {
        ...state,
        tabs: state.tabs.map((t) =>
          t.tabId === action.tabId
            ? { ...t, sessionId: action.sessionId, phase: 'running' as const }
            : t,
        ),
      };
    case 'TAB_SESSION_FAILED':
      return {
        ...state,
        tabs: state.tabs.map((t) =>
          t.tabId === action.tabId ? { ...t, phase: 'error' as const } : t,
        ),
      };
    case 'CLOSE_TAB': {
      const tabs = state.tabs.filter((t) => t.tabId !== action.tabId);
      let activeTabId = state.activeTabId;
      if (activeTabId === action.tabId) {
        activeTabId = tabs.length > 0 ? tabs[tabs.length - 1].tabId : null;
      }
      return { ...state, tabs, activeTabId };
    }
    case 'FOCUS_TAB':
      return { ...state, activeTabId: action.tabId, collapsed: false };
    case 'SET_TITLE':
      return {
        ...state,
        tabs: state.tabs.map((t) =>
          t.tabId === action.tabId ? { ...t, title: action.title } : t,
        ),
      };
    case 'TOGGLE_PANEL':
      return { ...state, collapsed: !state.collapsed };
    case 'TAB_ENDED':
      return {
        ...state,
        tabs: state.tabs.map((t) =>
          t.sessionId === action.sessionId
            ? { ...t, phase: 'closed' as const }
            : t,
        ),
      };
    case 'STARTUP_RECONCILE': {
      // Merge instead of replace: the IPC response is async, so the user
      // may have already opened a tab between provider mount and the
      // response resolving. Replacing state.tabs outright would clobber
      // that user-initiated tab (and its bindingRef entry). Append
      // restored tabs only when their backend session id is not already
      // represented in state — this also de-duplicates a session the
      // user opened via the panel against the same session the backend
      // still had alive from a previous run.
      const known = new Set<string>(
        state.tabs
          .map((t) => t.sessionId)
          .filter((sid): sid is string => typeof sid === 'string'),
      );
      const additions = action.tabs.filter((t) => {
        if (t.sessionId === null) return false;
        return !known.has(t.sessionId);
      });
      if (additions.length === 0) return state;
      return {
        ...state,
        tabs: [...state.tabs, ...additions],
      };
    }
    default:
      return state;
  }
}

export interface TerminalPanelOpenInput {
  machineId: string;
  machineLabel: string;
  projectId?: string;
  repoPath?: string;
  /**
   * Absolute working directory for the spawned shell. When supplied,
   * bypasses `resolve_repo_dir` entirely — used by `FeatureDetail` to
   * point the PTY at a feature worktree whose path is already absolute.
   * Spec §3 (c): feature terminals must target the supplied worktree.
   */
  workDir?: string;
  workBranch?: string | null;
}

export interface TerminalPanelContextValue {
  state: TerminalPanelState;
  open: (input: TerminalPanelOpenInput) => Promise<string>;
  close: (tabId: string) => Promise<void>;
  focus: (tabId: string) => void;
  setTitle: (tabId: string, title: string) => Promise<void>;
  togglePanel: () => void;
  /**
   * Resolve the backend `sess_*` id for a panel tab. Returns `null` if
   * the tab is still connecting, has been closed, or never existed.
   * Agent launchers (`AgentTerminalDrawer`) use this to grab the real
   * session id after `open()` resolves — `open()` itself only returns
   * the frontend-minted `tabId`, which the backend does not recognise.
   */
  getSessionId: (tabId: string) => string | null;
}

export const TerminalPanelContext = createContext<TerminalPanelContextValue | null>(null);

export interface TerminalPanelProviderProps {
  children: ReactNode;
}

function generateTabId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `tab_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 10)}`;
}

export function TerminalPanelProvider({ children }: TerminalPanelProviderProps) {
  const [state, dispatch] = useReducer(reducer, initialState);
  // Mirror the latest state into a ref so `open()` — which is `useCallback`-stable
  // — can read the current tab list synchronously without re-binding on every
  // reducer dispatch. Without this, an `open()` call would only ever see the
  // tabs present on first render and the dedupe-vs-create branch would never
  // reuse an existing tab after the user opened one. See `open()` for usage.
  const stateRef = useRef(state);
  stateRef.current = state;
  const bindingRef = useRef<Map<string, string>>(new Map());
  // Tabs whose start resolved after `close()` already removed them from
  // state. The cleanup branch in `open()` drains these on resolution so
  // we never leave a backend session with no UI owner.
  const cancelledRef = useRef<Set<string>>(new Set());

  const open = useCallback(
    async (input: TerminalPanelOpenInput): Promise<string> => {
      const existing = stateRef.current.tabs.find(
        (t) =>
          t.machineId === input.machineId &&
          (t.repoPath ?? null) === (input.repoPath ?? null) &&
          (t.workBranch ?? null) === (input.workBranch ?? null),
      );
      if (existing) {
        dispatch({ type: 'FOCUS_TAB', tabId: existing.tabId });
        return existing.tabId;
      }
      const tabId = generateTabId();
      cancelledRef.current.delete(tabId);
      const tab: TerminalTabDescriptor = {
        tabId,
        sessionId: null,
        machineId: input.machineId,
        machineLabel: input.machineLabel,
        projectId: input.projectId,
        repoPath: input.repoPath,
        workBranch: input.workBranch ?? null,
        title: '',
        phase: 'connecting',
        createdAt: Date.now(),
      };
      dispatch({ type: 'OPEN_TAB', tab });

      let resolveFailed = false;
      try {
        let workDir: string | undefined;
        if (input.workDir) {
          workDir = input.workDir;
        } else if (input.projectId && input.repoPath) {
          try {
            workDir = await resolveRepoDir(input.projectId, input.repoPath);
          } catch (err) {
            console.error(
              '[useTerminalPanel] resolveRepoDir failed; refusing to start the session:',
              err,
            );
            resolveFailed = true;
            dispatch({ type: 'TAB_SESSION_FAILED', tabId });
            throw err;
          }
        }

        // No seed channel: output produced before the surface attaches
        // accumulates in the backend scrollback ring and is replayed on
        // the first `attach_terminal_session` (TERMINALS_VIEW_SPEC §3).
        const sessionId = await startTerminalSession(
          input.machineId,
          workDir,
          input.workBranch ?? null,
        );

        if (cancelledRef.current.has(tabId)) {
          cancelledRef.current.delete(tabId);
          try {
            await closeTerminalSession(sessionId);
          } catch (cleanupErr) {
            console.warn(
              '[useTerminalPanel] close after close-during-connect race failed:',
              cleanupErr,
            );
          }
          return tabId;
        }

        bindingRef.current.set(tabId, sessionId);
        dispatch({ type: 'TAB_SESSION_ATTACHED', tabId, sessionId });
        return tabId;
      } catch (err) {
        cancelledRef.current.delete(tabId);
        if (!resolveFailed) {
          console.error('[useTerminalPanel] start_terminal_session failed:', err);
          dispatch({ type: 'TAB_SESSION_FAILED', tabId });
        }
        throw err;
      }
    },
    [],
  );

  const close = useCallback(async (tabId: string): Promise<void> => {
    const sessionId = bindingRef.current.get(tabId);
    bindingRef.current.delete(tabId);
    cancelledRef.current.delete(tabId);
    dispatch({ type: 'CLOSE_TAB', tabId });

    if (sessionId) {
      try {
        await closeTerminalSession(sessionId);
      } catch (err) {
        console.warn('[useTerminalPanel] close_terminal_session failed:', err);
      }
    } else {
      // start is still in flight; mark the tab cancelled so the
      // resolution branch of `open()` cleans the backend session up.
      cancelledRef.current.add(tabId);
    }
  }, []);

  const focus = useCallback((tabId: string): void => {
    dispatch({ type: 'FOCUS_TAB', tabId });
  }, []);

  const setTitle = useCallback(
    async (tabId: string, title: string): Promise<void> => {
      const sessionId = bindingRef.current.get(tabId);
      // Apply the title locally only after the backend has acknowledged
      // the rename. Without this ordering, a failed IPC would leave the
      // UI ahead of the backend, and the next `list_terminal_sessions`
      // reconcile would silently overwrite the user's title with the
      // stale backend value. For tabs whose `start` has not resolved yet
      // (no sessionId), we still commit locally — there is no backend
      // state to disagree with, and `open()` will replay the title when
      // its `start` IPC finishes.
      if (sessionId) {
        try {
          await renameTerminalSession(sessionId, title);
        } catch (err) {
          console.warn('[useTerminalPanel] rename_terminal_session failed:', err);
          throw err;
        }
      }
      dispatch({ type: 'SET_TITLE', tabId, title });
    },
    [],
  );

  const togglePanel = useCallback((): void => {
    dispatch({ type: 'TOGGLE_PANEL' });
  }, []);

  const getSessionId = useCallback(
    (tabId: string): string | null => {
      const fromBinding = bindingRef.current.get(tabId);
      if (fromBinding) return fromBinding;
      const fromState = state.tabs.find((t) => t.tabId === tabId);
      return fromState?.sessionId ?? null;
    },
    [state.tabs],
  );

  useTauriEvent<SessionInfo>('terminal-session-ended', (payload) => {
    dispatch({ type: 'TAB_ENDED', sessionId: payload.session_id });
  });

  // Startup reconciliation: rebuild tabs from any sessions the backend
  // survived from a prior webview reload / crash. We surface them as
  // `phase: 'closed'` so the user can close them (which forwards to
  // `close_terminal_session`, harmless if the backend already cleaned up).
  useEffect(() => {
    let cancelled = false;
    listTerminalSessions()
      .then((sessions) => {
        if (cancelled || sessions.length === 0) return;
        const tabs: TerminalTabDescriptor[] = sessions
          .slice()
          // Rust serialises from a HashMap, which has unstable order; sort
          // by `created_at` so restored tabs open in the order they were
          // originally created.
          .sort((a, b) => a.created_at - b.created_at)
          .map((s) => {
            const tabId = generateTabId();
            // Bind the tabId to the backend session id so close /
            // rename reach the backend. Without this, reconciled tabs
            // have no entry in `bindingRef` and the panel can't issue
            // IPC against them (spec feedback: "bind restored sessions
            // so close/rename reaches the backend").
            bindingRef.current.set(tabId, s.session_id);
            return {
              tabId,
              sessionId: s.session_id,
              machineId: s.machine_id,
              machineLabel: s.machine_id,
              title: s.title ?? s.machine_id,
              phase: 'closed' as const,
              createdAt: s.created_at * 1000,
            };
          });
        dispatch({ type: 'STARTUP_RECONCILE', tabs });
      })
      .catch((err) => {
        console.warn('[useTerminalPanel] startup list_terminal_sessions failed:', err);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const value = useMemo<TerminalPanelContextValue>(
    () => ({
      state,
      open,
      close,
      focus,
      setTitle,
      togglePanel,
      getSessionId,
    }),
    [
      state,
      open,
      close,
      focus,
      setTitle,
      togglePanel,
      getSessionId,
    ],
  );

  return (
    <TerminalPanelContext.Provider value={value}>
      {children}
    </TerminalPanelContext.Provider>
  );
}

export function useTerminalPanel(): TerminalPanelContextValue {
  const ctx = useContext(TerminalPanelContext);
  if (!ctx) {
    throw new Error('useTerminalPanel must be used within TerminalPanelProvider');
  }
  return ctx;
}
