/**
 * Block navigation away from a screen with unsaved work (task P3.3, audit
 * F38).
 *
 * The guard is installed on the navigation *context*, not on a component's
 * Back button, because F38 is the bug of covering one exit and missing the
 * others: `WorkflowEditor` dropped edits via its own Back arrow, the global
 * `Escape`, `Cmd+W`, and the mouse back button. All four end up in
 * `navigate` / `goBack` / `goForward`, so one guard there covers all four —
 * and any exit added later.
 *
 * Usage: keep the blocked intent in state, render a confirm prompt, then call
 * `proceed(intent)` once the user has saved or discarded.
 *
 * ```tsx
 * const [pending, setPending] = useState<NavigationIntent | null>(null);
 * const { proceed } = useNavigationGuard(dirty, setPending);
 * ```
 */
import { useEffect, useRef } from 'react';

import {
  useNavigation,
  type NavigationIntent,
} from '../context/NavigationContext';

export interface NavigationGuardHandle {
  /** Replay a blocked intent, bypassing all guards. */
  proceed: (intent: NavigationIntent) => void;
}

export function useNavigationGuard(
  /** Block while true. Flip to false (e.g. after saving) to let intents through. */
  active: boolean,
  /** Called with the intent that was blocked — prompt from here. */
  onBlocked: (intent: NavigationIntent) => void,
): NavigationGuardHandle {
  const { registerGuard, proceed } = useNavigation();

  // Latest callback without re-registering the guard on every render — a
  // re-register mid-navigation would drop the veto for that intent.
  const onBlockedRef = useRef(onBlocked);
  onBlockedRef.current = onBlocked;

  useEffect(() => {
    if (!active) return;
    return registerGuard((intent) => {
      onBlockedRef.current(intent);
      return false;
    });
  }, [active, registerGuard]);

  return { proceed };
}
