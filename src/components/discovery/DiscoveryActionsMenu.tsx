import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { MoreHorizontal } from 'lucide-react';

import { nextIndexForKey } from '../ui/rovingIndex';

export interface DiscoveryMenuAction {
  key: string;
  label: string;
  /** Shown under the label — on a disabled item, why it is disabled. */
  hint?: string | null;
  disabled?: boolean;
  /** A link item opens in a new window instead of calling `onSelect`. */
  href?: string;
  onSelect?: () => void;
  testId?: string;
}

interface DiscoveryActionsMenuProps {
  actions: DiscoveryMenuAction[];
}

/**
 * The Discovery header's secondary actions, once the header is too narrow to
 * carry them inline (`src/lib/discoveryHeaderLayout.ts`).
 *
 * Outside click, document-level Escape and focus restore follow
 * `src/components/AccountMenu.tsx`, whose comments carry why each is shaped the
 * way it is. With more than one item the `menu` role owes arrow keys, so they
 * move between the enabled items here.
 */
export function DiscoveryActionsMenu({ actions }: DiscoveryActionsMenuProps): React.ReactElement {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const itemRefs = useRef<Array<HTMLElement | null>>([]);
  const menuId = useId();

  const closeAndRestoreFocus = useCallback(() => {
    setOpen(false);
    triggerRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      closeAndRestoreFocus();
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [open, closeAndRestoreFocus]);

  const enabled = actions.map((action, i) => (action.disabled ? -1 : i)).filter((i) => i >= 0);

  const firstEnabled = enabled[0] ?? -1;
  useEffect(() => {
    if (open && firstEnabled >= 0) itemRefs.current[firstEnabled]?.focus();
  }, [open, firstEnabled]);

  function onMenuKeyDown(e: React.KeyboardEvent) {
    const from = enabled.indexOf(itemRefs.current.indexOf(document.activeElement as HTMLElement));
    const next = nextIndexForKey(e.key, from, enabled.length);
    if (next === null) return;
    e.preventDefault();
    itemRefs.current[enabled[next]]?.focus();
  }

  function select(action: DiscoveryMenuAction) {
    setOpen(false);
    action.onSelect?.();
  }

  const itemClass =
    'flex w-full flex-col items-start gap-0.5 rounded-md px-3 py-2 text-left text-[13px] text-slate-200 transition-colors hover:bg-white/[0.04] focus-visible:bg-white/[0.04] focus-visible:outline-none disabled:cursor-not-allowed disabled:text-slate-500 disabled:hover:bg-transparent';

  return (
    <div ref={containerRef} className="relative shrink-0">
      <button
        type="button"
        ref={triggerRef}
        onClick={() => setOpen((o) => !o)}
        aria-label="More discovery actions"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        data-testid="discovery-actions-trigger"
        className={`inline-flex h-10 w-10 items-center justify-center rounded-md border transition-colors hover:text-slate-100 ${
          open
            ? 'border-violet-500/45 bg-violet-500/10 text-slate-100'
            : 'border-white/5 text-slate-400 hover:border-white/15'
        }`}
      >
        <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
      </button>

      {open && (
        <div
          role="menu"
          id={menuId}
          aria-label="Discovery actions"
          data-testid="discovery-actions-menu"
          onKeyDown={onMenuKeyDown}
          className="glass-panel absolute right-0 top-full z-50 mt-2 flex w-72 flex-col rounded-lg border border-white/10 p-1.5 shadow-2xl"
        >
          {actions.map((action, i) => {
            const body = (
              <>
                <span>{action.label}</span>
                {action.hint && <span className="text-[11px] text-slate-500">{action.hint}</span>}
              </>
            );
            return action.href ? (
              <a
                key={action.key}
                ref={(el) => {
                  itemRefs.current[i] = el;
                }}
                role="menuitem"
                href={action.href}
                target="_blank"
                rel="noopener noreferrer"
                data-testid={action.testId}
                onClick={() => setOpen(false)}
                className={itemClass}
              >
                {body}
              </a>
            ) : (
              <button
                key={action.key}
                ref={(el) => {
                  itemRefs.current[i] = el;
                }}
                type="button"
                role="menuitem"
                disabled={action.disabled}
                data-testid={action.testId}
                onClick={() => select(action)}
                className={itemClass}
              >
                {body}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

export default DiscoveryActionsMenu;
