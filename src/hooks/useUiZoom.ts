import { useCallback, useEffect, useRef, useState } from 'react';

import { wheelPixels } from '../lib/canvasZoom';
import { uiZoomPref } from '../lib/uiPrefs';
import {
  DEFAULT_UI_ZOOM,
  IDLE_WHEEL_GESTURE,
  advanceWheelGesture,
  applyUiZoom,
  nextUiZoom,
  uiZoomKeyAction,
  type UiZoomAction,
} from '../lib/uiZoom';

export interface UiZoom {
  zoom: number;
  /** Counts the zoom actions the user has taken, including the ones that
   *  changed nothing: a press at either end of the ladder still has to be
   *  answered on screen, or it reads as a shortcut that does not work. */
  actions: number;
}

/**
 * Whole-window zoom, bound to the keyboard and the ctrl-wheel. Mount once —
 * the listeners are window-level. The decisions are `lib/uiZoom.ts`.
 *
 * **The key listener is on the capture phase and stops the event.** xterm
 * handles `keydown` on its own textarea and, off macOS, turns Ctrl+`-` into
 * `0x1f` for the shell — readline's undo. Heard on the way down, the chord
 * zooms the window from inside a terminal and the shell never sees it.
 *
 * **The wheel listener is on the bubble phase and yields to
 * `defaultPrevented`.** `useCanvasViewport` and React Flow both claim the
 * wheel over their own pane, and a pinch over a graph means "zoom the graph".
 * It is registered non-passive for the same reason `useCanvasViewport`'s is.
 */
export function useUiZoom(): UiZoom {
  const [zoom, setZoom] = useState(DEFAULT_UI_ZOOM);
  const [actions, setActions] = useState(0);
  const zoomRef = useRef(zoom);

  /** Set by the first user action. The stored value is read over IPC, so a
   *  keypress can beat it — and the answer must not then undo the keypress. */
  const acted = useRef(false);

  useEffect(() => {
    let cancelled = false;
    void uiZoomPref.read().then((stored) => {
      if (cancelled) return;
      if (acted.current) {
        // `write` was a no-op until this read armed it.
        uiZoomPref.write(zoomRef.current);
        return;
      }
      zoomRef.current = stored;
      setZoom(stored);
      if (stored !== DEFAULT_UI_ZOOM) void applyUiZoom(stored);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const act = useCallback((action: UiZoomAction) => {
    acted.current = true;
    setActions((count) => count + 1);
    const next = nextUiZoom(zoomRef.current, action);
    if (next === zoomRef.current) return;
    zoomRef.current = next;
    setZoom(next);
    uiZoomPref.write(next);
    void applyUiZoom(next);
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const action = uiZoomKeyAction(event);
      if (!action) return;
      event.preventDefault();
      event.stopPropagation();
      act(action);
    };

    let gesture = IDLE_WHEEL_GESTURE;
    const onWheel = (event: WheelEvent) => {
      if (!event.ctrlKey || event.defaultPrevented) return;
      event.preventDefault();
      const step = advanceWheelGesture(
        gesture,
        wheelPixels(event.deltaY, event.deltaMode),
        performance.now(),
      );
      gesture = step.gesture;
      if (step.action) act(step.action);
    };

    window.addEventListener('keydown', onKeyDown, { capture: true });
    window.addEventListener('wheel', onWheel, { passive: false });
    return () => {
      window.removeEventListener('keydown', onKeyDown, { capture: true });
      window.removeEventListener('wheel', onWheel);
    };
  }, [act]);

  return { zoom, actions };
}
