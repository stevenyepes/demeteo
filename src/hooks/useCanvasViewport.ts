import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';

import {
  anchoredScroll,
  clampZoom,
  fitZoom,
  steppedZoom,
  wheelPixels,
  wheelZoom,
  type Size,
} from '../lib/canvasZoom';

/**
 * Pan and zoom for the two canvases that are not React Flow. The arithmetic is
 * `lib/canvasZoom.ts`; what lives here is the part that needs a DOM.
 *
 * **Auto-fit stops the moment the operator zooms.** Refitting on every pane
 * resize is why the +/− buttons read as dead: a zoom past the pane's size
 * raises the scrollbars, a scrollbar shrinks the pane's content box, the
 * `ResizeObserver` fires, and the refit puts the zoom straight back where it
 * was — within one frame, so the button looks like it did nothing at all.
 * Framing on resize is only ever a default, and `fit()` re-arms it.
 *
 * The wheel listener is registered here rather than through `onWheel` because
 * React attaches wheel at the root as *passive*, where `preventDefault` is a
 * console warning and no more; the page would zoom or the pane would scroll
 * underneath the gesture.
 */
export interface CanvasViewport {
  zoom: number;
  zoomIn: () => void;
  zoomOut: () => void;
  /** Frame the content and re-arm the follow-the-pane default. */
  fit: () => void;
  reset: () => void;
  /** Ref for the scrolling pane — the element the gestures are read from. */
  paneRef: (element: HTMLDivElement | null) => void;
  /** Ref for the scaled canvas inside it, which a cursor-anchored zoom
   *  measures from: its edge already carries any centring margin. */
  canvasRef: (element: HTMLDivElement | null) => void;
  panning: boolean;
  panProps: {
    onPointerDown: (event: React.PointerEvent<HTMLDivElement>) => void;
    onPointerMove: (event: React.PointerEvent<HTMLDivElement>) => void;
    onPointerUp: (event: React.PointerEvent<HTMLDivElement>) => void;
    onPointerCancel: (event: React.PointerEvent<HTMLDivElement>) => void;
  };
}

interface Drag {
  pointerId: number;
  x: number;
  y: number;
  left: number;
  top: number;
}

/** A pointer that started on one of these is operating a control, not the
 *  background, so it must not become a pan — the node cards are buttons. */
const INTERACTIVE = 'button, a, input, textarea, select, [role="button"]';

export function useCanvasViewport(content: Size): CanvasViewport {
  const [pane, setPane] = useState<HTMLDivElement | null>(null);
  const [canvas, setCanvas] = useState<HTMLDivElement | null>(null);
  const [zoom, setZoom] = useState(1);
  const [panning, setPanning] = useState(false);

  /** Whether a pane resize still gets to reframe the canvas. A ref, not state:
   *  nothing renders differently either way, and the resize callback has to
   *  read it at tick time — an observer armed while it was `true` outlives the
   *  zoom that revoked it. */
  const followsPane = useRef(true);

  const zoomRef = useRef(zoom);
  zoomRef.current = zoom;
  const contentRef = useRef(content);
  contentRef.current = content;

  /** Scroll offsets an anchored zoom owes the pane, applied once the scaled
   *  canvas has the size the new zoom gives it. */
  const pendingScroll = useRef<{ left: number; top: number } | null>(null);
  const drag = useRef<Drag | null>(null);

  const fit = useCallback(() => {
    if (!pane) return;
    followsPane.current = true;
    setZoom(
      fitZoom({ width: pane.clientWidth, height: pane.clientHeight }, contentRef.current),
    );
  }, [pane]);

  const zoomAbout = useCallback(
    (next: number, clientX: number, clientY: number) => {
      const current = zoomRef.current;
      if (next === current) return;
      if (pane && canvas) {
        const rect = canvas.getBoundingClientRect();
        pendingScroll.current = {
          left: anchoredScroll(pane.scrollLeft, clientX - rect.left, current, next),
          top: anchoredScroll(pane.scrollTop, clientY - rect.top, current, next),
        };
      }
      followsPane.current = false;
      setZoom(next);
    },
    [pane, canvas],
  );

  /** The pane's centre is what a button press holds still — the operator is
   *  looking at the middle of the pane, not at wherever the pointer rested. */
  const zoomFromCentre = useCallback(
    (next: number) => {
      if (!pane) {
        followsPane.current = false;
        setZoom(next);
        return;
      }
      const rect = pane.getBoundingClientRect();
      zoomAbout(next, rect.left + rect.width / 2, rect.top + rect.height / 2);
    },
    [pane, zoomAbout],
  );

  const zoomIn = useCallback(
    () => zoomFromCentre(steppedZoom(zoomRef.current, 1)),
    [zoomFromCentre],
  );
  const zoomOut = useCallback(
    () => zoomFromCentre(steppedZoom(zoomRef.current, -1)),
    [zoomFromCentre],
  );
  const reset = useCallback(() => zoomFromCentre(clampZoom(1)), [zoomFromCentre]);

  const contentKey = `${content.width}x${content.height}`;

  // `clientWidth`/`clientHeight` are read from the element rather than from
  // `entry.contentRect` because jsdom's `ResizeObserverStub` fires with an
  // empty entry list — the same reason `useHeaderDensity` reads `offsetWidth`.
  useEffect(() => {
    if (!pane || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      if (!followsPane.current) return;
      setZoom(
        fitZoom({ width: pane.clientWidth, height: pane.clientHeight }, contentRef.current),
      );
    });
    observer.observe(pane);
    return () => observer.disconnect();
  }, [pane, contentKey]);

  useEffect(() => {
    if (!pane) return;
    function onWheel(event: WheelEvent) {
      event.preventDefault();
      const current = zoomRef.current;
      zoomAbout(
        wheelZoom(current, wheelPixels(event.deltaY, event.deltaMode)),
        event.clientX,
        event.clientY,
      );
    }
    pane.addEventListener('wheel', onWheel, { passive: false });
    return () => pane.removeEventListener('wheel', onWheel);
  }, [pane, zoomAbout]);

  useLayoutEffect(() => {
    const target = pendingScroll.current;
    pendingScroll.current = null;
    if (!target || !pane) return;
    pane.scrollLeft = target.left;
    pane.scrollTop = target.top;
  }, [zoom, pane]);

  const onPointerDown = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (!pane || event.button > 1) return;
      if (event.button === 0 && (event.target as Element).closest(INTERACTIVE)) return;
      // Without this the drag paints a text selection across every card it
      // crosses, and the pan reads as a botched click.
      event.preventDefault();
      drag.current = {
        pointerId: event.pointerId,
        x: event.clientX,
        y: event.clientY,
        left: pane.scrollLeft,
        top: pane.scrollTop,
      };
      pane.setPointerCapture?.(event.pointerId);
      setPanning(true);
    },
    [pane],
  );

  const onPointerMove = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const start = drag.current;
      if (!start || !pane || start.pointerId !== event.pointerId) return;
      pane.scrollLeft = start.left - (event.clientX - start.x);
      pane.scrollTop = start.top - (event.clientY - start.y);
    },
    [pane],
  );

  const endPan = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (!drag.current || drag.current.pointerId !== event.pointerId) return;
      // Releasing a capture that was never taken throws — and it is never
      // taken when the pointer is gone by the time the up arrives.
      if (pane?.hasPointerCapture?.(event.pointerId)) pane.releasePointerCapture(event.pointerId);
      drag.current = null;
      setPanning(false);
    },
    [pane],
  );

  return {
    zoom,
    zoomIn,
    zoomOut,
    fit,
    reset,
    paneRef: setPane,
    canvasRef: setCanvas,
    panning,
    panProps: {
      onPointerDown,
      onPointerMove,
      onPointerUp: endPan,
      onPointerCancel: endPan,
    },
  };
}
