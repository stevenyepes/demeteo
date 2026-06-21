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
