import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';

import {
  DEFAULT_MIN_PRIMARY,
  DEFAULT_MIN_SECONDARY,
  maxSecondaryWidth,
  resolveSecondaryWidth,
  secondaryWidthForKey,
  secondaryWidthFromPointer,
  type SplitBounds,
} from './splitPaneGeometry';

/** Custom property the panes size off. Written imperatively during a drag; the
 *  plan (UI_REDESIGN_PLAN §4.1) names it `--inspector-w` for the pipeline
 *  view's use of it, spelled generically here because the primitive knows
 *  nothing about inspectors. */
export const SPLIT_SECONDARY_VAR = '--split-secondary-w';

export interface SplitPaneProps {
  primary: ReactNode;
  secondary: ReactNode;
  /** Committed width of the secondary pane in px, clamped on render into what
   *  the container can actually seat. */
  secondaryWidth: number;
  /** Called once per drag, on release — never per pointer move. */
  onSecondaryWidthCommit: (width: number) => void;
  minPrimary?: number;
  minSecondary?: number;
  /** Accessible name for the divider; never rendered as text. */
  label?: string;
  className?: string;
}

interface Drag {
  pointerId: number;
  /** Right edge of the container, read once at pointer-down: the box cannot
   *  change under a drag that only moves the divider inside it. */
  containerRight: number;
  bounds: SplitBounds;
  width: number;
}

/**
 * Two panes side by side with a draggable, keyboard-operable divider, where the
 * secondary pane's width is owned by the caller.
 *
 * **A drag sets no React state.** Each pointer move resolves a width and writes
 * it to `SPLIT_SECONDARY_VAR` on the container element through a ref; React
 * hears about it once, on release, through `onSecondaryWidthCommit`. The mock
 * this replaces called `setState` per mouse-move, and in this app the primary
 * pane holds the ELK-laid-out run graph whose column `useRunColumnLayout`
 * observes and feeds into layout planning — so a width routed through React
 * would re-plan graph layout at pointer frequency. `SplitPane.test.tsx` pins
 * the property; the geometry it drags through lives in `splitPaneGeometry.ts`.
 *
 * The committed width is *not* owned here, so a caller is free to hold it in a
 * reducer, a stored preference, or nothing at all.
 */
export function SplitPane({
  primary,
  secondary,
  secondaryWidth,
  onSecondaryWidthCommit,
  minPrimary = DEFAULT_MIN_PRIMARY,
  minSecondary = DEFAULT_MIN_SECONDARY,
  label = 'Resize panes',
  className = '',
}: SplitPaneProps): React.ReactElement {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const handleRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<Drag | null>(null);
  /** Held in state only for what has to be *rendered* from it — the divider's
   *  value range, and a committed width the container has since outgrown. Every
   *  interaction below measures the box itself instead, since a bound one
   *  observer tick stale would clamp a live drag to the wrong number. */
  const [containerWidth, setContainerWidth] = useState(0);

  const boundsFor = useCallback(
    (width: number): SplitBounds => ({ containerWidth: width, minPrimary, minSecondary }),
    [minPrimary, minSecondary],
  );

  const applyWidth = useCallback((width: number) => {
    containerRef.current?.style.setProperty(SPLIT_SECONDARY_VAR, `${width}px`);
    handleRef.current?.setAttribute('aria-valuenow', String(width));
  }, []);

  /** The committed width is the caller's, but the primary pane's minimum
   *  outranks it: a window narrowed after the width was committed would
   *  otherwise starve the primary pane until the user dragged again. Nothing is
   *  committed back for it, so the caller's width returns when the box does. */
  const renderedWidth = resolveSecondaryWidth(secondaryWidth, boundsFor(containerWidth));

  useLayoutEffect(() => {
    applyWidth(renderedWidth);
  }, [renderedWidth, applyWidth]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const read = () => {
      const { width } = el.getBoundingClientRect();
      setContainerWidth((prev) => (prev === width ? prev : width));
    };
    read();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(read);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const clearDrag = useCallback((pointerId: number) => {
    dragRef.current = null;
    const handle = handleRef.current;
    if (handle?.hasPointerCapture?.(pointerId)) handle.releasePointerCapture?.(pointerId);
    containerRef.current?.style.removeProperty('user-select');
  }, []);

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const container = containerRef.current;
    if (!container) return;
    const box = container.getBoundingClientRect();
    /** `preventDefault` keeps the drag from selecting text, and with it the
     *  compatibility mousedown that would have focused the divider — so the
     *  focus that makes the arrow keys work after a drag is taken by hand. */
    event.preventDefault();
    event.currentTarget.focus({ preventScroll: true });
    /** jsdom implements none of the pointer-capture methods; called optionally
     *  so the drag is testable there, and left as the only capture mechanism so
     *  a real drag survives the pointer leaving the window. */
    event.currentTarget.setPointerCapture?.(event.pointerId);
    dragRef.current = {
      pointerId: event.pointerId,
      containerRight: box.right,
      bounds: boundsFor(box.width),
      width: secondaryWidth,
    };
    container.style.setProperty('user-select', 'none');
  };

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || event.pointerId !== drag.pointerId) return;
    const next = resolveSecondaryWidth(
      secondaryWidthFromPointer(event.clientX, drag.containerRight),
      drag.bounds,
    );
    if (next === drag.width) return;
    drag.width = next;
    applyWidth(next);
  };

  const commitDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || event.pointerId !== drag.pointerId) return;
    clearDrag(drag.pointerId);
    if (drag.width !== secondaryWidth) onSecondaryWidthCommit(drag.width);
  };

  const revertDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || event.pointerId !== drag.pointerId) return;
    clearDrag(drag.pointerId);
    applyWidth(renderedWidth);
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const container = containerRef.current;
    if (!container) return;
    const next = secondaryWidthForKey(
      event.key,
      secondaryWidth,
      boundsFor(container.getBoundingClientRect().width),
    );
    if (next === null) return;
    event.preventDefault();
    if (next !== secondaryWidth) onSecondaryWidthCommit(next);
  };

  const valueMax = maxSecondaryWidth(boundsFor(containerWidth));
  const valueMin = Math.min(minSecondary, valueMax);

  return (
    <div
      ref={containerRef}
      data-testid="split-pane"
      className={`relative grid h-full min-h-0 w-full ${className}`}
      style={{ gridTemplateColumns: `minmax(0, 1fr) var(${SPLIT_SECONDARY_VAR}, 0px)` }}
    >
      <div className="min-w-0 min-h-0 overflow-hidden">{primary}</div>
      <div data-testid="split-pane-secondary" className="min-w-0 min-h-0 overflow-hidden">
        {secondary}
      </div>
      <div
        ref={handleRef}
        data-testid="split-pane-handle"
        role="separator"
        aria-orientation="vertical"
        aria-label={label}
        aria-valuenow={renderedWidth}
        aria-valuemin={valueMin}
        aria-valuemax={valueMax > 0 ? valueMax : undefined}
        tabIndex={0}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={commitDrag}
        onLostPointerCapture={commitDrag}
        onPointerCancel={revertDrag}
        onKeyDown={onKeyDown}
        /* `w-2.5` is hit area, not appearance — the visible line is the 1px
           child, and a 1px pointer target is a usability defect. */
        className="group absolute inset-y-0 z-20 flex w-2.5 translate-x-1/2 cursor-col-resize items-center justify-center bg-transparent transition-colors hover:bg-cyan-500/20 focus-visible:bg-cyan-500/20 focus-visible:outline-none active:bg-cyan-500/20"
        style={{ right: `var(${SPLIT_SECONDARY_VAR}, 0px)` }}
      >
        <span className="h-full w-px bg-white/10 transition-colors group-hover:bg-cyan-500/50 group-focus-visible:bg-cyan-500/50" />
      </div>
    </div>
  );
}

export default SplitPane;
