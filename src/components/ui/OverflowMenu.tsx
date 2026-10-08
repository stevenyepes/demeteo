import React, { useEffect, useId, useRef, useState } from 'react';
import { MoreHorizontal } from 'lucide-react';

export interface OverflowMenuItem {
  label: string;
  onSelect: () => void;
  title?: string;
  disabled?: boolean;
}

export interface OverflowMenuProps {
  /** Accessible name of the trigger — it shows only an ellipsis. */
  label: string;
  items: OverflowMenuItem[];
  className?: string;
}

/**
 * A `⋯` button holding the actions a surface offers but rarely needs.
 *
 * Dismissal listens on `pointerdown` at the document, not `click`, so a press
 * that starts outside closes the menu before whatever it lands on reacts.
 * Escape is handled on the menu's own subtree rather than the window: the run
 * view already has window-level key handlers (`useRunShortcuts`, the overlays),
 * and a second one here would answer the same Escape they do.
 */
export function OverflowMenu({ label, items, className = '' }: OverflowMenuProps): React.ReactElement | null {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuId = useId();

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (event.target instanceof Node && rootRef.current?.contains(event.target)) return;
      setOpen(false);
    };
    document.addEventListener('pointerdown', onPointerDown);
    return () => document.removeEventListener('pointerdown', onPointerDown);
  }, [open]);

  if (items.length === 0) return null;

  return (
    <div
      ref={rootRef}
      className={`relative ${className}`}
      onKeyDown={(event) => {
        if (event.key !== 'Escape' || !open) return;
        event.stopPropagation();
        setOpen(false);
        triggerRef.current?.focus();
      }}
    >
      <button
        ref={triggerRef}
        type="button"
        aria-label={label}
        title={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        onClick={() => setOpen((o) => !o)}
        className="flex h-8 w-8 items-center justify-center rounded-lg border border-white/10 bg-white/[0.03] text-slate-400 transition hover:bg-white/[0.08] hover:text-white"
      >
        <MoreHorizontal className="h-4 w-4" />
      </button>
      {open && (
        <div
          id={menuId}
          role="menu"
          className="absolute right-0 top-full z-30 mt-1.5 min-w-48 rounded-lg border border-white/10 bg-[var(--bg-sidebar)] p-1 shadow-xl shadow-black/50 animate-fade-in"
        >
          {items.map((item) => (
            <button
              key={item.label}
              type="button"
              role="menuitem"
              title={item.title}
              disabled={item.disabled}
              onClick={() => {
                setOpen(false);
                item.onSelect();
              }}
              className="block w-full rounded-md px-3 py-2 text-left text-xs text-slate-200 transition hover:bg-white/[0.06] disabled:opacity-40"
            >
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
