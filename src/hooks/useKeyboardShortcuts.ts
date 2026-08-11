import { useEffect, useRef } from 'react';

import { isEditableTarget } from '../lib/shortcuts';

interface ShortcutMap {
  onNewProject?: () => void;
  onNewFeature?: () => void;
  /** Opens the New-terminal launcher (Cmd/Ctrl+T). */
  onNewTerminal?: () => void;
  onOpenSettings?: () => void;
  onOpenCommandPalette?: () => void;
  onOpenDocs?: () => void;
  onToggleSidebar?: () => void;
  onEscape?: () => void;
  onNavigateProject?: (index: number) => void;
  onCloseCurrentView?: () => void;
  onNextFeature?: () => void;
  onPreviousFeature?: () => void;
  /** Opens the full-page Terminals view. Named "open" rather than
   *  "toggle": the navigation reducer dedups a repeat navigation, so the
   *  chord only ever opens the view, it does not toggle back. */
  onOpenTerminalsView?: () => void;
}

export function useKeyboardShortcuts(handlers: ShortcutMap) {
  const ref = useRef(handlers);
  ref.current = handlers;

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      const h = ref.current;
      const mod = e.metaKey || e.ctrlKey;

      if (e.key === 'Escape' && h.onEscape) {
        h.onEscape();
        return;
      }

      if (e.key === 'F1') {
        e.preventDefault();
        h.onOpenDocs?.();
        return;
      }

      // `?` is a character, so this dispatcher may not consume it out of a
      // field the user is typing into — `preventDefault` below would otherwise
      // swallow the keystroke and open the docs panel over their work.
      if (e.key === '?' && !mod && !e.shiftKey && !e.altKey && !isEditableTarget(e.target)) {
        e.preventDefault();
        h.onOpenDocs?.();
        return;
      }

      if (!mod) return;

      switch (e.key) {
        case 'k':
        case 'K':
          e.preventDefault();
          h.onOpenCommandPalette?.();
          break;
        case 'n':
          if (e.shiftKey) {
            e.preventDefault();
            h.onNewFeature?.();
          } else {
            e.preventDefault();
            h.onNewProject?.();
          }
          break;
        case 't':
        case 'T':
          // Bare Cmd/Ctrl+T opens New Feature; Cmd/Ctrl+Shift+T is left to
          // the webview (reopen-closed-tab), so we don't touch the Shift case.
          if (!e.shiftKey) {
            e.preventDefault();
            h.onNewFeature?.();
          }
          break;
        case 'w':
        case 'W':
          e.preventDefault();
          h.onCloseCurrentView?.();
          break;
        case 'f':
        case 'F':
          if (e.shiftKey) {
            e.preventDefault();
            h.onOpenCommandPalette?.();
          }
          break;
        case 'g':
        case 'G':
          e.preventDefault();
          if (e.shiftKey) {
            h.onPreviousFeature?.();
          } else {
            h.onNextFeature?.();
          }
          break;
        case ',':
          e.preventDefault();
          h.onOpenSettings?.();
          break;
        case 'b':
        case 'B':
          e.preventDefault();
          h.onToggleSidebar?.();
          break;
        case '.':
          e.preventDefault();
          h.onOpenCommandPalette?.();
          break;
        case '?':
          e.preventDefault();
          h.onOpenDocs?.();
          break;
        case '`':
        case '~':
          // Cmd/Ctrl + ` opens the full-page Terminals view
          // (TERMINALS_VIEW_SPEC §4.3 / §6). Sessions stay alive as the
          // user navigates — the view is a pure renderer of session
          // lifecycle, not its owner.
          //
          // Cmd/Ctrl + Shift + ` opens the New-terminal launcher on that
          // view. Shift + backtick emits '~' on most layouts, so we accept
          // either key and branch on the Shift flag rather than the char.
          e.preventDefault();
          if (e.shiftKey) {
            h.onNewTerminal?.();
          } else {
            h.onOpenTerminalsView?.();
          }
          break;
        default:
          if (e.key >= '1' && e.key <= '9') {
            e.preventDefault();
            h.onNavigateProject?.(parseInt(e.key) - 1);
          }
          break;
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, []);
}
