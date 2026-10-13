import { useEffect, useState } from 'react';

import { useUiZoom } from '../hooks/useUiZoom';
import { uiZoomPercent } from '../lib/uiZoom';

export const UI_ZOOM_INDICATOR_MS = 1400;

/**
 * Owns the window zoom (`useUiZoom`) and names the level for a moment after
 * each change. Mount exactly once, anywhere — it needs no provider.
 *
 * The readout is why this is a component and not a bare hook: a rung is a
 * small change, and at either end of the ladder a press changes nothing at
 * all, so without a number on screen neither can be told from a shortcut that
 * did not fire. `CanvasZoomControls` carries a permanent readout for the same
 * reason; this one is transient because the window has no toolbar to keep it in.
 */
export function UiZoomIndicator(): React.ReactElement | null {
  const { zoom, actions } = useUiZoom();
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    if (actions === 0) return;
    setVisible(true);
    const timer = setTimeout(() => setVisible(false), UI_ZOOM_INDICATOR_MS);
    return () => clearTimeout(timer);
  }, [actions]);

  if (!visible) return null;

  return (
    <div
      role="status"
      aria-live="polite"
      data-testid="ui-zoom-indicator"
      className="glass-panel animate-fade-in pointer-events-none fixed left-1/2 top-6 z-[90] -translate-x-1/2 px-4 py-2 font-mono text-sm text-slate-100"
    >
      <span className="sr-only">Zoom </span>
      {uiZoomPercent(zoom)}
    </div>
  );
}
