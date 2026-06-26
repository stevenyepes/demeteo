import React, { useEffect } from 'react';

interface ModalProps {
  onClose?: () => void;
  children: React.ReactNode;
  className?: string;
  backdropClassName?: string;
}

export function Modal({ onClose, children, className = '', backdropClassName = '' }: ModalProps) {
  useEffect(() => {
    if (!onClose) return;
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  return (
    <div
      className={`fixed inset-0 z-50 flex items-center justify-center bg-[#08090c]/80 backdrop-blur-sm ${backdropClassName}`}
      onClick={(e) => { if (e.target === e.currentTarget) onClose?.(); }}
    >
      <div className={className}>
        {children}
      </div>
    </div>
  );
}
