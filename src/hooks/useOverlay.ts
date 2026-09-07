import { useEffect, useId } from 'react';

import { useOptionalUIState } from '../context';

/**
 * Declare that this component is an overlay, for as long as it is mounted
 * (audit F35).
 *
 * Registering makes every Back gesture — `Escape`, the mouse back button,
 * `Alt+←` — stand down while it is open, so closing a dialog no longer *also*
 * navigates the view underneath it. It does not close anything: an overlay
 * already owns its own dismissal, and a registry that also held closers would
 * be a second place for that to be decided.
 *
 * Call it unconditionally at the top of a component that is only rendered when
 * open. An overlay that renders itself hidden must not call it — it would
 * register while invisible and swallow Escape for a dialog nobody can see.
 */
export function useOverlay(): void {
  // One id per mount, so two of the same dialog are two entries and the first
  // to unmount cannot deregister the second.
  const id = useId();
  // Optional on purpose — see `useOptionalUIState` for why absence is not a
  // mistake here.
  const uiDispatch = useOptionalUIState()?.uiDispatch;

  useEffect(() => {
    if (!uiDispatch) return;
    uiDispatch({ type: 'PUSH_OVERLAY', id });
    return () => uiDispatch({ type: 'POP_OVERLAY', id });
  }, [id, uiDispatch]);
}
