// Cross-component state for the keyboard-shortcut help overlay.
//
// The `ShortcutHelp` overlay is intentionally self-installing — it installs
// its own keydown listener for `F1` / `?` and owns its open/closed state in
// `useState`. That keeps the overlay usable without any wiring in App.tsx.
//
// `ShortcutsProvider` exists for *cross-component* state: any other React
// component (e.g. a top-bar "?" button, a footer hint, or a future toast)
// needs a stable entry point to programmatically open the help, and the
// help overlay itself needs a stable way to know whether something else
// has forced the panel open.
//
// The two layers communicate through a `CustomEvent` on `window` so the
// provider does NOT have to wrap the whole tree. The bridge is:
//
//   • `useShortcuts().openHelp()`     → dispatches `demeteo:help-open`
//   • `useShortcuts().closeHelp()`    → dispatches `demeteo:help-close`
//   • `useShortcuts().toggleHelp()`   → dispatches `demeteo:help-toggle`
//   • `<ShortcutHelp />` listens for  → these three event names
//
// If no `ShortcutsProvider` is mounted, `useShortcuts()` throws — by
// design, since every consumer should be wired through it (mirrors the
// convention used by `useUIState`).

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from 'react';
import type { ReactNode } from 'react';

/** Dispatched on `window` whenever a component wants the help overlay to open. */
export const SHORTCUTS_HELP_OPEN_EVENT = 'demeteo:help-open';
/** Dispatched on `window` whenever something wants to dismiss the overlay. */
export const SHORTCUTS_HELP_CLOSE_EVENT = 'demeteo:help-close';
/** Dispatched on `window` to flip the overlay open ↔ closed. */
export const SHORTCUTS_HELP_TOGGLE_EVENT = 'demeteo:help-toggle';

export interface ShortcutsContextValue {
  /** Whether the help overlay is currently visible. */
  isHelpOpen: boolean;
  /** Force the overlay open. The overlay closes itself on Esc / backdrop. */
  openHelp: () => void;
  /** Force the overlay closed. */
  closeHelp: () => void;
  /** Flip the overlay's open state. */
  toggleHelp: () => void;
}

const ShortcutsContext = createContext<ShortcutsContextValue | null>(null);

export interface ShortcutsProviderProps {
  children: ReactNode;
}

export function ShortcutsProvider({ children }: ShortcutsProviderProps): React.ReactElement {
  const [isHelpOpen, setHelpOpen] = useState(false);

  // External "force open / close / toggle" via CustomEvent keeps the
  // overlay decoupled from the provider (so the overlay can self-install
  // without `App.tsx` putting a provider above it).
  useEffect(() => {
    const onOpen = (): void => setHelpOpen(true);
    const onClose = (): void => setHelpOpen(false);
    const onToggle = (): void => setHelpOpen((prev) => !prev);
    window.addEventListener(SHORTCUTS_HELP_OPEN_EVENT, onOpen);
    window.addEventListener(SHORTCUTS_HELP_CLOSE_EVENT, onClose);
    window.addEventListener(SHORTCUTS_HELP_TOGGLE_EVENT, onToggle);
    return () => {
      window.removeEventListener(SHORTCUTS_HELP_OPEN_EVENT, onOpen);
      window.removeEventListener(SHORTCUTS_HELP_CLOSE_EVENT, onClose);
      window.removeEventListener(SHORTCUTS_HELP_TOGGLE_EVENT, onToggle);
    };
  }, []);

  const openHelp = useCallback((): void => {
    window.dispatchEvent(new CustomEvent(SHORTCUTS_HELP_OPEN_EVENT));
  }, []);
  const closeHelp = useCallback((): void => {
    window.dispatchEvent(new CustomEvent(SHORTCUTS_HELP_CLOSE_EVENT));
  }, []);
  const toggleHelp = useCallback((): void => {
    window.dispatchEvent(new CustomEvent(SHORTCUTS_HELP_TOGGLE_EVENT));
  }, []);

  const value = useMemo<ShortcutsContextValue>(
    () => ({ isHelpOpen, openHelp, closeHelp, toggleHelp }),
    [isHelpOpen, openHelp, closeHelp, toggleHelp],
  );

  return (
    <ShortcutsContext.Provider value={value}>
      {children}
    </ShortcutsContext.Provider>
  );
}

export function useShortcuts(): ShortcutsContextValue {
  const ctx = useContext(ShortcutsContext);
  if (!ctx) {
    throw new Error(
      'useShortcuts must be used within ShortcutsProvider — wrap the tree once at the app root.',
    );
  }
  return ctx;
}

/**
 * Soft variant: returns `null` if no provider is installed.
 * Useful for components that prefer to no-op rather than throw, e.g.
 * the `<ShortcutHelp />` overlay itself when it self-installs in an
 * un-wrapped subtree.
 */
export function useShortcutsOptional(): ShortcutsContextValue | null {
  return useContext(ShortcutsContext);
}
