import { useEffect } from 'react';
import { useNavigation, useUIState } from '../context';
import { hasEscapeOverlay } from '../lib/escapeLadder';

/**
 * Window-level mouse back/forward bridge.
 *
 * Listens for `mousedown` events with `button === 3` (XButton1 / mouse back)
 * and `button === 4` (XButton2 / mouse forward) at the window level and
 * delegates to the in-app navigation stack exposed by `useNavigation()`.
 *
 * This hook never calls `history.back()` / `history.forward()` — the
 * in-app stack is the single source of truth for navigation history, even
 * inside the Tauri webview where the browser navigation API would otherwise
 * observe the same input.
 *
 * The default browser-back behaviour is suppressed via `preventDefault()` so
 * the webview cannot fall back to its own history traversal in response to
 * the same button press.
 *
 * **Suppressed while an overlay is open**, which `lib/shortcuts.ts` has
 * documented since it was written and nothing implemented: the press used to
 * go straight past a modal to the view underneath it, so dismissing a dialog
 * with the mouse moved the app instead. The ladder decides, rather than a
 * second list of conditions here — it is the same question Escape asks, and
 * two answers to it is how the pair drift apart.
 *
 * Self-mounting: drop a single `<MouseNavigationBridge />` anywhere inside
 * `<NavigationProvider>` (typically near the app root) and the listener is
 * installed for the lifetime of that mount.
 */
export function useMouseNavigation(): void {
  const { view, goBack, goForward } = useNavigation();
  const { ui } = useUIState();

  useEffect(() => {
    const handleMouseDown = (event: MouseEvent): void => {
      if (event.button !== 3 && event.button !== 4) return;
      event.preventDefault();
      if (hasEscapeOverlay(ui, view)) return;
      if (event.button === 3) {
        goBack();
      } else {
        goForward();
      }
    };

    window.addEventListener('mousedown', handleMouseDown);
    return () => {
      window.removeEventListener('mousedown', handleMouseDown);
    };
  }, [ui, view, goBack, goForward]);
}

/**
 * Self-mounting component variant of {@link useMouseNavigation}. Render a
 * single instance inside `<NavigationProvider>` to install the window-level
 * mouse back/forward listener — no props or wiring required.
 */
export function MouseNavigationBridge(): null {
  useMouseNavigation();
  return null;
}
