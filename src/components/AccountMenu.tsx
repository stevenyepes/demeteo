import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { Settings } from 'lucide-react';

import type { Provider } from '../types';

// The header's account control: the avatar is the trigger and Settings lives in
// the menu it opens, rather than in a standalone ⚙ beside it (spec D1 — "below
// the Terminals option" is this panel, opening under the end of the nav row, not
// a second header band costing permanent vertical space).
//
// The menu carries the provider identity line and Settings, and nothing else. A
// sign-out / switch-account / profile item would each need a backend command
// that does not exist, and a menu entry that cannot do its job is worse than an
// absent one.

export interface AccountMenuProps {
  connectedProvider: Provider | null;
  onNavigateSettings: () => void;
}

export function AccountMenu({
  connectedProvider,
  onNavigateSettings,
}: AccountMenuProps): React.ReactElement {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const firstItemRef = useRef<HTMLButtonElement>(null);
  const menuId = useId();

  const closeAndRestoreFocus = useCallback(() => {
    setOpen(false);
    triggerRef.current?.focus();
  }, []);

  // Outside click closes the menu (`NotificationBell`'s handler shape). The
  // listener exists only while open, so a closed menu costs nothing on every
  // mousedown in the app.
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

  // Escape is listened for on the document, not the container, because focus is
  // reachably *outside* the container while the menu is open: clicking the
  // identity line — a non-focusable `<div>` — blurs to `<body>`, which the
  // mousedown handler above leaves open (the target is inside the container) and
  // which no container-scoped React handler can see. WKWebView reaches that state
  // with no click at all: Safari does not focus a `<button>` on mousedown, so a
  // container-scoped handler would be dead from the moment the menu opens on the
  // macOS target of the `build.yml` matrix, where no gate in this repo runs.
  //
  // `stopPropagation` keeps Escape off the global shortcut handler, which is bound
  // to `window` (`useKeyboardShortcuts.ts`) and so sits one hop further up the
  // bubble path than this one.
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

  // What earns the `menu` role rather than claiming it: opening moves focus onto
  // the first item, so the menu is operable from the keyboard alone. With one
  // item there is nothing for arrow keys to move between; a second item is the
  // point at which they have to be added or the role given up.
  useEffect(() => {
    if (open) firstItemRef.current?.focus();
  }, [open]);

  const username = connectedProvider?.username ?? '';

  return (
    <div ref={containerRef} className="relative shrink-0">
      <button
        type="button"
        ref={triggerRef}
        onClick={() => setOpen((o) => !o)}
        className="flex items-center rounded-full transition-colors hover:opacity-90 focus-visible:outline focus-visible:outline-1 focus-visible:outline-cyan-400"
        title={username ? `Account — ${username}` : 'Account'}
        aria-label={username ? `Account — ${username}` : 'Account'}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        data-testid="topbar-account-trigger"
      >
        {connectedProvider?.avatarUrl ? (
          <img
            src={connectedProvider.avatarUrl}
            alt={connectedProvider.username}
            className="w-8 h-8 rounded-full border-2 border-cyan-500/50 object-cover"
          />
        ) : (
          <span
            data-testid="topbar-account-avatar-fallback"
            className="w-8 h-8 rounded-full bg-gradient-to-tr from-violet-600 to-cyan-600 border-2 border-white/10"
          />
        )}
      </button>

      {open && (
        <div
          data-testid="topbar-account-menu"
          className="glass-panel absolute right-0 top-full mt-2 w-56 rounded-lg border border-white/10 shadow-2xl overflow-hidden z-50"
        >
          {/* Outside the `menu` role deliberately: a screen reader announcing a
              menu of two items, one of which is inert text, is worse than the
              identity line being read as the panel's own content. */}
          <div
            data-testid="topbar-account-identity"
            className="px-3 py-2 border-b border-white/5"
          >
            <div className="text-xs text-slate-200 truncate">
              {connectedProvider ? connectedProvider.username : 'No provider connected'}
            </div>
            <div className="text-[10px] font-mono text-slate-500 truncate">
              {connectedProvider ? connectedProvider.host : 'Connect one in Settings'}
            </div>
          </div>
          <div role="menu" id={menuId}>
            <button
              type="button"
              role="menuitem"
              ref={firstItemRef}
              // Restore before navigating: activating unmounts the element holding
              // focus, so the trigger has to claim it back or `activeElement` falls
              // to `<body>` and the next Tab restarts at the top of the view just
              // arrived at. Ordering leaves a settings view free to take focus itself.
              onClick={() => {
                closeAndRestoreFocus();
                onNavigateSettings();
              }}
              className="w-full flex items-center gap-2 px-3 py-2 text-left text-xs text-slate-300 hover:bg-white/5 hover:text-white transition-colors"
              data-testid="topbar-account-settings"
            >
              <Settings className="w-4 h-4 shrink-0 text-cyan-400" />
              <span>Settings</span>
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

export default AccountMenu;
