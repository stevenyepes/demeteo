import { Maximize2, ZoomIn, ZoomOut } from 'lucide-react';

import { ZOOM_MAX, ZOOM_MIN, zoomPercent } from '../../lib/canvasZoom';
import type { CanvasViewport } from '../../hooks/useCanvasViewport';

/**
 * The zoom cluster both hand-rolled canvases wear. One component so the two
 * cannot drift, and one grouped pill rather than three loose buttons so it
 * reads as a single control the way `WorkflowCanvas`'s does.
 *
 * The readout is the reason this is not just two buttons: at the clamp a press
 * is a no-op, and with nothing on screen naming the current scale there is no
 * way to tell that from a control that does not work. It doubles as the reset.
 */
export function CanvasZoomControls({
  viewport,
  className = '',
}: {
  viewport: CanvasViewport;
  className?: string;
}): React.ReactElement {
  const { zoom, zoomIn, zoomOut, fit, reset } = viewport;

  return (
    <div
      data-testid="canvas-zoom-controls"
      className={`flex items-center overflow-hidden rounded-full border border-white/5 bg-slate-900/90 backdrop-blur-md ${className}`}
    >
      <button
        type="button"
        aria-label="Zoom out"
        title="Zoom out"
        disabled={zoom <= ZOOM_MIN}
        onClick={zoomOut}
        className="flex h-7 w-7 items-center justify-center text-slate-300 transition-colors hover:bg-slate-800/70 hover:text-white disabled:opacity-40 disabled:hover:bg-transparent"
      >
        <ZoomOut className="h-3.5 w-3.5" />
      </button>
      <button
        type="button"
        aria-label="Reset zoom to 100%"
        title="Reset zoom · scroll to zoom, drag to pan"
        onClick={reset}
        className="min-w-[3.25rem] px-1 py-1 text-center font-mono text-[10px] text-slate-300 tabular-nums transition-colors hover:bg-slate-800/70 hover:text-white"
      >
        {zoomPercent(zoom)}
      </button>
      <button
        type="button"
        aria-label="Zoom in"
        title="Zoom in"
        disabled={zoom >= ZOOM_MAX}
        onClick={zoomIn}
        className="flex h-7 w-7 items-center justify-center text-slate-300 transition-colors hover:bg-slate-800/70 hover:text-white disabled:opacity-40 disabled:hover:bg-transparent"
      >
        <ZoomIn className="h-3.5 w-3.5" />
      </button>
      <span aria-hidden="true" className="h-4 w-px bg-white/10" />
      <button
        type="button"
        aria-label="Fit to view"
        title="Fit to view"
        onClick={fit}
        className="flex h-7 w-7 items-center justify-center text-slate-300 transition-colors hover:bg-slate-800/70 hover:text-white"
      >
        <Maximize2 className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
