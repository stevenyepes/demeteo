import { useEffect, useRef } from 'react';

interface ShortcutMap {
  onNewProject?: () => void;
  onNewFeature?: () => void;
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

      if (e.key === '?' && !mod && !e.shiftKey && !e.altKey) {
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
          // Cmd/Ctrl + ` opens the full-page Terminals view
          // (TERMINALS_VIEW_SPEC §4.3 / §6). Sessions stay alive as the
          // user navigates — the view is a pure renderer of session
          // lifecycle, not its owner.
          e.preventDefault();
          h.onOpenTerminalsView?.();
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
