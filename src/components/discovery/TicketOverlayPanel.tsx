import React, { useEffect } from 'react';
import { OverlayPortal } from '../ui/OverlayPortal';

interface TicketOverlayPanelProps {
  widthPx: number;
  onClose: () => void;
  label: string;
  children: React.ReactNode;
}

// Docked variant of `Modal.tsx`'s portal + Escape idiom, but not its backdrop:
// `DISCOVERY_UI_SPEC.md` §3.2.1 describes this panel as floating over the
// ticket pane rather than displacing it, so — unlike `Modal.tsx` — the wrapper
// stays `pointer-events-none` and carries no dimming/blur. `InterviewColumn`
// and `TicketColumn` remain mounted underneath at `fixed inset-y-0 right-0`'s
// left edge, in normal flow, and must stay clickable; only the docked panel
// itself re-enables pointer events. With no exposed backdrop region left to
// click, there is no click-to-dismiss — Escape is the only dismiss gesture.
export function TicketOverlayPanel({ widthPx, onClose, label, children }: TicketOverlayPanelProps) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  return (
    <OverlayPortal>
      <div className="pointer-events-none fixed inset-0 z-50" aria-label={label}>
        <div
          className="pointer-events-auto fixed inset-y-0 right-0 shadow-[0_0_32px_rgba(0,0,0,0.5)]"
          style={{ width: `min(92vw, ${widthPx}px)` }}
        >
          {children}
        </div>
      </div>
    </OverlayPortal>
  );
}
