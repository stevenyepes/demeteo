// Self-installing keyboard-shortcut help overlay.
//
// Mount a single `<ShortcutHelp />` anywhere in the React tree (no App.tsx
// wiring required, no provider required) and you get:
//
//   • A `F1` and `?` window-level keydown listener that opens the overlay
//     in a styled modal. Existing per-component ESC handlers (e.g. the
//     settings modal) remain unaffected — the overlay's own ESC handler
//     calls `event.stopPropagation()` so the global dispatcher in
//     `useKeyboardShortcuts` does not double-fire.
//
//   • A glassmorphism panel grouped by `ShortcutCategory`, drawn with the
//     AGENTS.md §5 tokens (`rgba(18,22,30,0.75)` surface, `#8b5cf6`
//     violet accents, backdrop-blur for depth).
//
//   • A "Quick Reference" callout at the top of the overlay highlighting
//     the two most useful global shortcuts — `Cmd/Ctrl + K` for the
//     command palette and `Cmd/Ctrl + `` for the terminal panel — so
//     new users see them without having to scan the registry below.
//
//   • A `?help-open` / `?help-close` / `?help-toggle` CustomEvent bridge
//     so other components can drive the panel without prop-drilling or a
//     provider chain. See `ShortcutsContext.tsx` for the symmetric side.
//
// Visual design follows AGENTS.md §5:
//   - background: `#08090c` / `#0d0f14`
//   - card surface: `rgba(18,22,30,0.75)` + `backdrop-blur`
//   - accents: violet `#8b5cf6` for active / running indicators, cyan /
//     emerald for status badges.

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { createPortal } from 'react-dom';
import { X, Keyboard, Sparkles, TerminalSquare, Command } from 'lucide-react';

import {
  SHORTCUTS,
  SHORTCUT_GROUPS,
  findShortcutById,
  formatEntryChords,
  isEditableTarget,
  type ShortcutBadge,
  type ShortcutEntry,
  type ShortcutPlatform,
} from '../lib/shortcuts';
import {
  SHORTCUTS_HELP_CLOSE_EVENT,
  SHORTCUTS_HELP_OPEN_EVENT,
  SHORTCUTS_HELP_TOGGLE_EVENT,
} from '../context/ShortcutsContext';

// Platform detection — `navigator.platform` is deprecated but remains
// the broadest signal inside a Tauri webview where user agent strings
// are unreliable. Default to "other" so the docs / universal formatter
// fallback is correct on Linux + Windows + Linux-Tauri.

function detectPlatform(): ShortcutPlatform {
  if (typeof navigator === 'undefined') return 'other';
  const ua = navigator.platform || '';
  if (/Mac|iPhone|iPad/i.test(ua)) return 'mac';
  return 'other';
}

const BADGE_CLASSES: Record<ShortcutBadge, string> = {
  deprecated:
    'bg-amber-500/10 text-amber-300 border-amber-500/30',
  'intentionally-ignored':
    'bg-slate-500/10 text-slate-400 border-slate-500/20',
  alias:
    'bg-cyan-500/10 text-cyan-300 border-cyan-500/30',
};

const BADGE_LABEL: Record<ShortcutBadge, string> = {
  deprecated: 'Deprecated',
  'intentionally-ignored': 'Intentionally ignored',
  alias: 'Alias',
};

/**
 * Two-callout "Quick Reference" rendered at the top of the help overlay.
 * The full registry below stays the source of truth, but a new user
 * benefits from seeing the two most-used global shortcuts front and
 * centre (spec §3 (f)): the command palette and the terminal panel.
 */
function QuickReferenceCallout({ platform }: { platform: ShortcutPlatform }): React.ReactElement | null {
  const palette = findShortcutById('cmd-k-command-palette');
  const terminalPanel = findShortcutById('cmd-backtick-toggle-terminal-panel');
  if (!palette && !terminalPanel) return null;
  return (
    <div
      data-testid="shortcut-help-quick-reference"
      className="mx-4 mt-4 mb-2 grid grid-cols-1 md:grid-cols-2 gap-2 rounded-xl border border-white/[0.08] bg-gradient-to-r from-violet-500/[0.06] to-cyan-500/[0.06] p-3"
    >
      <div className="flex items-start gap-2 min-w-0">
        <Command className="w-4 h-4 text-violet-300 mt-0.5 shrink-0" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-xs">
            <span className="text-slate-200 font-medium">Command palette</span>
            {palette && (
              <kbd className="ml-auto shrink-0 text-[10px] font-mono text-violet-200 bg-violet-500/10 border border-violet-500/30 px-1.5 py-0.5 rounded shadow-inner whitespace-nowrap">
                {formatEntryChords(palette, platform)}
              </kbd>
            )}
          </div>
          <p className="text-[11px] text-slate-400 leading-snug mt-0.5">
            Fuzzy launcher for every action in the app.
          </p>
        </div>
      </div>
      <div className="flex items-start gap-2 min-w-0">
        <TerminalSquare className="w-4 h-4 text-cyan-300 mt-0.5 shrink-0" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-xs">
            <span className="text-slate-200 font-medium">Open Terminals view</span>
            {terminalPanel && (
              <kbd className="ml-auto shrink-0 text-[10px] font-mono text-cyan-200 bg-cyan-500/10 border border-cyan-500/30 px-1.5 py-0.5 rounded shadow-inner whitespace-nowrap">
                {formatEntryChords(terminalPanel, platform)}
              </kbd>
            )}
          </div>
          <p className="text-[11px] text-slate-400 leading-snug mt-0.5">
            Open the full-page Terminals view — sessions stay alive as you navigate.
          </p>
        </div>
      </div>
    </div>
  );
}

interface ShortcutRowProps {
  entry: ShortcutEntry;
  platform: ShortcutPlatform;
}

function ShortcutRow({ entry, platform }: ShortcutRowProps): React.ReactElement {
  return (
    <div
      className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 gap-y-1 py-3 px-4 rounded-lg hover:bg-white/[0.02] transition-colors items-center"
      data-testid={`shortcut-row-${entry.id}`}
    >
      <div className="flex flex-col gap-0.5 min-w-0">
        <div className="flex items-center gap-2 min-w-0">
          <span className="text-sm font-medium text-slate-200 truncate">
            {entry.label}
          </span>
          {entry.badge && (
            <span
              className={`shrink-0 text-[10px] uppercase tracking-wider font-semibold px-1.5 py-0.5 rounded border ${BADGE_CLASSES[entry.badge]}`}
            >
              {BADGE_LABEL[entry.badge]}
            </span>
          )}
        </div>
        <span className="text-xs text-slate-400 leading-snug">
          {entry.description}
        </span>
      </div>
      <div className="flex flex-col items-end gap-1 shrink-0">
        {entry.chords.length === 0 ? (
          <span className="text-xs text-slate-500 italic font-mono">—</span>
        ) : (
          entry.chords.map((chord, index) => (
            <kbd
              key={index}
              className="text-[11px] font-mono text-violet-200 bg-violet-500/10 border border-violet-500/30 px-2 py-1 rounded shadow-inner whitespace-nowrap"
            >
              {formatEntryChords({ ...entry, chords: [chord] }, platform)}
            </kbd>
          ))
        )}
      </div>
    </div>
  );
}

export interface ShortcutHelpProps {
  /**
   * Optional custom portal target. When `undefined` the overlay renders
   * into `document.body` (creating the typical modal-portal behaviour).
   */
  container?: Element | null;
  /**
   * Force the platform that chord glyphs render for. Defaults to platform
   * auto-detection at mount time. Tests typically pin this to `'mac'` or
   * `'other'` for snapshot stability.
   */
  platform?: ShortcutPlatform;
}

export function ShortcutHelp(props: ShortcutHelpProps): React.ReactElement | null {
  const [isOpen, setIsOpen] = useState(false);
  const [platform, setPlatform] = useState<ShortcutPlatform>(
    props.platform ?? 'other',
  );
  // Tight ref so the keydown listener always sees the latest state
  // without having to re-install itself on every render.
  const stateRef = useRef({ isOpen, platform });
  stateRef.current = { isOpen, platform };

  const close = useCallback((): void => setIsOpen(false), []);

  // Auto-detect platform on mount if no override was provided.
  useEffect(() => {
    if (props.platform === undefined) {
      setPlatform(detectPlatform());
    }
  }, [props.platform]);

  // External open / close / toggle via CustomEvent bridge.
  useEffect(() => {
    const onOpen = (): void => setIsOpen(true);
    const onClose = (): void => setIsOpen(false);
    const onToggle = (): void => setIsOpen((prev) => !prev);
    window.addEventListener(SHORTCUTS_HELP_OPEN_EVENT, onOpen);
    window.addEventListener(SHORTCUTS_HELP_CLOSE_EVENT, onClose);
    window.addEventListener(SHORTCUTS_HELP_TOGGLE_EVENT, onToggle);
    return () => {
      window.removeEventListener(SHORTCUTS_HELP_OPEN_EVENT, onOpen);
      window.removeEventListener(SHORTCUTS_HELP_CLOSE_EVENT, onClose);
      window.removeEventListener(SHORTCUTS_HELP_TOGGLE_EVENT, onToggle);
    };
  }, []);

  // Window-level keydown listener for F1 / ? .
  //
  // The listener is intentionally permissive and matches the registry
  // definitions of `f1-help` and `question-mark-help` (which are
  // strict: primary:false, shift:false, alt:false). When the overlay
  // is already open we still consume `Escape` here so the help panel
  // is closed from the global dispatcher — per-modal handlers must
  // call `event.stopPropagation()` if they want to opt out.
  useEffect(() => {
    const handleKey = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') {
        if (stateRef.current.isOpen) {
          event.preventDefault();
          event.stopPropagation();
          setIsOpen(false);
        }
        return;
      }

      if (stateRef.current.isOpen) {
        return;
      }

      // While a text field is focused we don't want to compete with it
      // for keystrokes (typing "?" in an input must not open help).
      if (isEditableTarget(event.target)) {
        return;
      }

      const mod = event.metaKey || event.ctrlKey;
      const alt = event.altKey;
      const shift = event.shiftKey;

      // F1: no modifier.
      if (event.key === 'F1' && !mod && !alt && !shift) {
        event.preventDefault();
        setIsOpen(true);
        return;
      }

      // ? (literal punctuation, no modifier).
      if (event.key === '?' && !mod && !alt) {
        event.preventDefault();
        setIsOpen(true);
        return;
      }
    };

    window.addEventListener('keydown', handleKey);
    return () => {
      window.removeEventListener('keydown', handleKey);
    };
  }, []);

  // Body scroll lock while the panel is open. Set via the helper local
  // `overflow` rather than fighting React state into portal subtree.
  useEffect(() => {
    if (!isOpen) return;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, [isOpen]);

  // Stable container resolution: prefer an explicit container prop, fall
  // back to `document.body`. Memoised so the portal doesn't re-mount on
  // re-renders when the body reference is stable (it always is in a Tauri
  // webview, but tests pass a container).
  const portalContainer = useMemo<Element | null>(() => {
    if (props.container !== undefined) return props.container;
    if (typeof document === 'undefined') return null;
    return document.body;
  }, [props.container]);

  // Avoid running the listener / portal logic on the server.
  if (typeof window === 'undefined') return null;

  if (!portalContainer) return null;

  if (!isOpen) {
    return null;
  }

  const panel = (
    <div
      data-testid="shortcut-help-overlay"
      className="fixed inset-0 z-[80] flex items-center justify-center p-4 sm:p-8"
      role="dialog"
      aria-modal="true"
      aria-labelledby="shortcut-help-title"
    >
      <div
        className="absolute inset-0 bg-[#08090c]/80 backdrop-blur-md"
        onClick={close}
        data-testid="shortcut-help-backdrop"
      />
      <div
        className="relative w-full max-w-5xl max-h-[88vh] flex flex-col rounded-2xl border border-white/[0.08] shadow-2xl overflow-hidden"
        style={{
          background: 'rgba(18,22,30,0.75)',
          backdropFilter: 'blur(12px)',
          WebkitBackdropFilter: 'blur(12px)',
        }}
        onClick={(event) => event.stopPropagation()}
      >
        {/* ── Header ────────────────────────────────────────────── */}
        <header className="flex items-center justify-between gap-4 px-6 py-4 border-b border-white/5 bg-[#0d0f14]/60 shrink-0">
          <div className="flex items-center gap-3">
            <div className="w-9 h-9 rounded-xl bg-violet-500/10 border border-violet-500/30 flex items-center justify-center">
              <Keyboard className="w-5 h-5 text-violet-300" />
            </div>
            <div className="flex flex-col leading-tight">
              <h2
                id="shortcut-help-title"
                className="text-lg font-bold font-heading text-white"
              >
                Keyboard &amp; Mouse Shortcuts
              </h2>
              <span className="text-xs text-slate-400 flex items-center gap-1.5">
                <Sparkles className="w-3 h-3 text-cyan-400" />
                Press <kbd className="font-mono text-violet-300">F1</kbd> or{' '}
                <kbd className="font-mono text-violet-300">?</kbd> to reopen
              </span>
            </div>
          </div>
          <button
            type="button"
            onClick={close}
            className="p-2 text-slate-400 hover:text-white rounded-lg hover:bg-white/5 transition-colors shrink-0"
            aria-label="Close shortcut help"
            data-testid="shortcut-help-close"
          >
            <X className="w-5 h-5" />
          </button>
        </header>

        {/* ── Body (grouped) ───────────────────────────────────── */}
        <div
          className="flex-1 overflow-y-auto px-6 py-5 bg-[#0a0c12]/30"
          data-testid="shortcut-help-body"
        >
          <QuickReferenceCallout platform={platform} />
          <div className="grid gap-5 md:grid-cols-2 mt-4">
            {SHORTCUT_GROUPS.map((group) => {
              if (group.entries.length === 0) return null;
              return (
                <section
                  key={group.id}
                  className="rounded-xl border border-white/5 bg-[#0d0f14]/40 overflow-hidden"
                  data-testid={`shortcut-help-group-${group.id}`}
                >
                  <header className="px-4 py-3 border-b border-white/5 bg-gradient-to-r from-violet-500/[0.07] to-transparent">
                    <h3 className="text-xs font-bold uppercase tracking-[0.18em] text-violet-300 font-heading">
                      {group.title}
                    </h3>
                    {group.description && (
                      <p className="text-[11px] text-slate-500 mt-0.5">
                        {group.description}
                      </p>
                    )}
                  </header>
                  <div className="divide-y divide-white/[0.04]">
                    {group.entries.map((entry) => (
                      <ShortcutRow
                        key={entry.id}
                        entry={entry}
                        platform={platform}
                      />
                    ))}
                  </div>
                </section>
              );
            })}
          </div>
        </div>

        {/* ── Footer (escape hint) ─────────────────────────────── */}
        <footer className="px-6 py-3 border-t border-white/5 bg-[#0d0f14]/60 flex items-center justify-between text-xs text-slate-500 shrink-0">
          <span>
            {SHORTCUTS.length} shortcuts ·{' '}
            {SHORTCUT_GROUPS.reduce((acc, group) => acc + group.entries.length, 0)} entries across {SHORTCUT_GROUPS.length} groups
          </span>
          <span className="flex items-center gap-1.5">
            Press{' '}
            <kbd className="font-mono text-violet-300">Esc</kbd>
            to close
          </span>
        </footer>
      </div>
    </div>
  );

  return createPortal(panel, portalContainer);
}

/**
 * Self-mounting component variant of `ShortcutHelp`. Renders a single
 * `<ShortcutHelp />` with no props — drop one of these anywhere in the
 * React tree (typically near the root) and the overlay is installed.
 *
 * This is the public surface other components should reach for.
 */
export function ShortcutHelpBridge(): React.ReactElement | null {
  return <ShortcutHelp />;
}
