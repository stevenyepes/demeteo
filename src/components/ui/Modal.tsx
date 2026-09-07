import React, { useEffect } from 'react';
import { useOverlay } from '../../hooks/useOverlay';
import { OverlayPortal } from './OverlayPortal';

interface ModalProps {
  onClose?: () => void;
  children: React.ReactNode;
  className?: string;
  backdropClassName?: string;
}

export function Modal({ onClose, children, className = '', backdropClassName = '' }: ModalProps) {
  // Every `Modal` is an overlay, so registering here covers all of them at
  // once — and a dialog added later is covered the day it is written, which a
  // per-modal flag never was (audit F35).
  useOverlay();

  useEffect(() => {
    if (!onClose) return;
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  // Portalled so the backdrop covers the whole window rather than only the
  // content area — see `OverlayPortal` for why being inside <main> isn't enough.
  return (
    <OverlayPortal>
      <div
        className={`fixed inset-0 z-50 flex items-center justify-center bg-[#08090c]/80 backdrop-blur-sm ${backdropClassName}`}
        onClick={(e) => { if (e.target === e.currentTarget) onClose?.(); }}
      >
        <div className={className}>
          {children}
        </div>
      </div>
    </OverlayPortal>
  );
}
