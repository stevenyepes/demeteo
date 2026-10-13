import React, { useEffect } from 'react';
import { useOverlay } from '../../hooks/useOverlay';

interface TicketOverlayPanelProps {
  onClose: () => void;
  label: string;
  children: React.ReactNode;
}

// Floats over the ticket pane rather than displacing it (`DISCOVERY_UI_SPEC.md`
// §3.2.1), and is anchored to the workspace row, not the window. We tried a
// portal pinned to the window's full height: the inspector then covered the app
// header in this mode but sat beneath it in three-up, so one click produced two
// layouts depending on whether the interview was showing. Anchored here, the
// two modes differ only in whether the panel pushes the graph or floats over
// it. The caller must render this inside a `relative` box. No backdrop and no
// click-to-dismiss — the interview and graph stay live beside it — so Escape
// is the only dismiss gesture.
export function TicketOverlayPanel({ onClose, label, children }: TicketOverlayPanelProps) {
  // This component exists only in the overlaid layouts — three-up renders the
  // same pane inline, where it is a column and not an overlay — so registering
  // here says "an overlay is open" exactly when one is.
  useOverlay();

  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  return (
    <aside
      aria-label={label}
      className="absolute inset-y-0 right-0 z-30 flex max-w-full shadow-[-24px_0_48px_rgba(0,0,0,0.55)]"
    >
      {children}
    </aside>
  );
}
