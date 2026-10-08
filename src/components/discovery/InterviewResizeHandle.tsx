import React, { useRef } from 'react';

import { interviewWidthForKey, resolveInterviewWidth } from './discoveryLayout';

/** The custom property the interview column sizes off. */
export const INTERVIEW_WIDTH_VAR = '--interview-w';

interface InterviewResizeHandleProps {
  /** The workspace row: the box the width is measured against, and the element
   *  the live width is written onto. */
  rowEl: HTMLDivElement | null;
  /** The width the interview renders at now, already resolved. */
  width: number;
  /** Called once per drag, on release, and once per keystroke. */
  onCommit: (width: number) => void;
}

interface Drag {
  pointerId: number;
  rowLeft: number;
  rowWidth: number;
  width: number;
}

/**
 * The divider on the interview's right edge.
 *
 * **A drag sets no React state** — the same discipline as `SplitPane`, for the
 * same reason: each pointer move writes `INTERVIEW_WIDTH_VAR` straight onto the
 * row, and React hears about the width once, on release. Routed through state,
 * a drag would re-render the transcript and the ticket graph at pointer
 * frequency. `SplitPane` itself does not fit here: it sizes the pane on its
 * *right*, and the interview is on the left.
 */
export function InterviewResizeHandle({
  rowEl,
  width,
  onCommit,
}: InterviewResizeHandleProps): React.ReactElement {
  const dragRef = useRef<Drag | null>(null);

  const apply = (next: number) => rowEl?.style.setProperty(INTERVIEW_WIDTH_VAR, `${next}px`);

  const finish = (target: HTMLDivElement, pointerId: number) => {
    dragRef.current = null;
    if (target.hasPointerCapture?.(pointerId)) target.releasePointerCapture?.(pointerId);
    rowEl?.style.removeProperty('user-select');
  };

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || !rowEl) return;
    event.preventDefault();
    event.currentTarget.focus({ preventScroll: true });
    event.currentTarget.setPointerCapture?.(event.pointerId);
    const box = rowEl.getBoundingClientRect();
    dragRef.current = { pointerId: event.pointerId, rowLeft: box.left, rowWidth: box.width, width };
    rowEl.style.setProperty('user-select', 'none');
  };

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || event.pointerId !== drag.pointerId) return;
    const next = resolveInterviewWidth(event.clientX - drag.rowLeft, drag.rowWidth);
    if (next === drag.width) return;
    drag.width = next;
    apply(next);
  };

  const commit = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || event.pointerId !== drag.pointerId) return;
    finish(event.currentTarget, drag.pointerId);
    if (drag.width !== width) onCommit(drag.width);
  };

  const revert = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || event.pointerId !== drag.pointerId) return;
    finish(event.currentTarget, drag.pointerId);
    apply(width);
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const next = interviewWidthForKey(event.key, width, rowEl?.getBoundingClientRect().width ?? 0);
    if (next === null) return;
    event.preventDefault();
    if (next !== width) onCommit(next);
  };

  return (
    <div className="relative z-20 w-0 shrink-0">
      <div
        data-testid="interview-resize"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the interview"
        aria-valuenow={width}
        tabIndex={0}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={commit}
        onLostPointerCapture={commit}
        onPointerCancel={revert}
        onKeyDown={onKeyDown}
        /* `w-2.5` is hit area, not appearance — the visible line is the 1px child. */
        className="group absolute inset-y-0 -left-1.5 flex w-2.5 cursor-col-resize justify-center transition-colors hover:bg-cyan-500/20 focus-visible:bg-cyan-500/20 focus-visible:outline-none active:bg-cyan-500/20"
      >
        <span className="h-full w-px bg-transparent transition-colors group-hover:bg-cyan-500/50 group-focus-visible:bg-cyan-500/50" />
      </div>
    </div>
  );
}

export default InterviewResizeHandle;
