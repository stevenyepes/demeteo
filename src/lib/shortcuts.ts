// Single source of truth for every keyboard shortcut Demeteo supports.
//
// Two complementary views live here:
//
//   1. A *declarative* registry (`SHORTCUTS` + `SHORTCUT_GROUPS`) that the
//      `ShortcutHelp` overlay renders — adds, removes, and badge changes
//      happen in exactly one place and the help panel + the docs file stay
//      consistent automatically.
//
//   2. A *procedural* matcher (`matchesKeyEvent` / `matchesMouseButton`)
//      for any consumer that wants to ask "did the user just press this
//      chord?" without re-implementing modifier-key plumbing.
//
// The runtime keyboard dispatcher stays in `src/hooks/useKeyboardShortcuts.ts`
// and mouse back/forward in `src/hooks/useMouseNavigation.ts` — those own
// the imperative bindings. This file is the declarative catalogue the rest
// of the app queries.
//
// Symbol conventions (mac / other, universal):
//   ⌘    — Command    (mac)
//   ⌃    — Control    (other)
//   ⇧    — Shift      (both)
//   ⌥    — Option/Alt (both)
//
// Modifier semantics (strict, exhaustive):
//   Every chord declares the *exact* modifier state it requires: each of
//   `primary`, `shift`, `alt` is a boolean that must equal the matching
//   `KeyboardEvent` flag. There is intentionally **no** "don't care" mode
//   — the matcher being permissive in the past caused chords to fire
//   twice (e.g. `Cmd+?` matching the help chord in addition to the docs
//   chord). Defaults are picked by the registry, not by the matcher.
//
// The registry intentionally documents `Cmd/Ctrl + Shift + T` as
// `intentionally-ignored` so the help panel does not appear to silently
// drop the chord that the user gets when they mash the muscle-memory
// "reopen closed tab" combo.

export type ShortcutCategory =
  | 'navigation'
  | 'feature'
  | 'project'
  | 'view'
  | 'palette'
  | 'data'
  | 'help'
  | 'mouse';

export type ShortcutBadge = 'deprecated' | 'intentionally-ignored' | 'alias';

export interface ShortcutChord {
  /**
   * Cmd (mac) / Ctrl (else). `true` = must hold Cmd/Ctrl; `false` = must
   * NOT. The matcher collapses `metaKey || ctrlKey` so the same chord
   * works against either physical key.
   */
  primary: boolean;
  /** Shift. `true` = must be held; `false` = must NOT be held. */
  shift: boolean;
  /** Alt / Option. `true` = must be held; `false` = must NOT be held. */
  alt: boolean;

  /**
   * Key identifier. Case-insensitive for single-character keys ('t' and
   * 'T' are equivalent). Punctuation characters are matched verbatim
   * (',' '.' '?' '[' ']'). Special keys use the `event.key` convention:
   * 'F1', 'F11', 'Escape', 'ArrowLeft', 'ArrowRight', 'Enter'.
   *
   * Mouse-button chords use the synthetic keys 'MouseButton3' (XButton1 /
   * back) and 'MouseButton4' (XButton2 / forward).
   */
  key: string;
}

export interface ShortcutEntry {
  /** Stable identifier used by tests, analytics, and deep links. */
  id: string;
  /**
   * Ordered list of chords that activate this action. An empty array is a
   * valid value for `intentionally-ignored` placeholders (no chord fires
   * the action, but the help panel still surfaces it).
   */
  chords: readonly ShortcutChord[];
  /** Human label, e.g. "New Feature". */
  label: string;
  /** One-line description for the help overlay. */
  description: string;
  /** Group/section rendered in the help overlay. */
  category: ShortcutCategory;
  /** Optional badge drawn next to the entry in the help overlay. */
  badge?: ShortcutBadge;
}

export interface ShortcutGroup {
  id: ShortcutCategory;
  title: string;
  /** Section copy rendered above the entry list. */
  description?: string;
  entries: readonly ShortcutEntry[];
}

// ── The registry ────────────────────────────────────────────────────────
//
// Ordering is intentional and stable: keep the help overlay's column
// ordering deterministic for visual diff-review.
//
// Modifier discipline: every chord declares primary/shift/alt as strict
// booleans. Anything that reads as "no modifier required" spells
// `shift: false, alt: false` explicitly — leaving the modifier off the
// literal is the most common source of double-firing across two
// shortcut layers, so we never allow it in the source form.

const NEW_FEATURE_CHORDS: readonly ShortcutChord[] = [
  { primary: true, shift: false, alt: false, key: 't' },
];

const NEW_FEATURE_ALIAS_CHORDS: readonly ShortcutChord[] = [
  { primary: true, shift: true, alt: false, key: 'n' },
];

const NEW_TERMINAL_CHORDS: readonly ShortcutChord[] = [
  { primary: true, shift: true, alt: false, key: '`' },
];

const PROJECT_NUMBER_ENTRIES: readonly ShortcutEntry[] = (() => {
  // Generate Cmd/Ctrl+1..9 entries individually so the help overlay renders
  // each row, rather than eliding them behind a generic token.
  const entries: ShortcutEntry[] = [];
  for (let n = 1 as 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9; n <= 9; n = (n + 1) as 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9) {
    const k = String(n);
    entries.push({
      id: `cmd-${k}-switch-project-${n}`,
      chords: [{ primary: true, shift: false, alt: false, key: k }],
      label: `Switch to project #${n}`,
      description:
        n === 1
          ? 'Jump to the first project in the sidebar.'
          : `Jump directly to project #${n} in the sidebar.`,
      category: 'project',
    });
  }
  return entries;
})();

export const SHORTCUTS: readonly ShortcutEntry[] = [
  // ── feature ──
  {
    id: 'cmd-t-new-feature',
    chords: NEW_FEATURE_CHORDS,
    label: 'New Feature',
    description: 'Open the Start Feature modal inside the current project.',
    category: 'feature',
  },
  {
    id: 'cmd-shift-n-new-feature-alias',
    chords: NEW_FEATURE_ALIAS_CHORDS,
    label: 'New Feature',
    description: 'Deprecated alias for `Cmd/Ctrl + T`. Kept for muscle memory.',
    category: 'feature',
    badge: 'deprecated',
  },
  {
    id: 'cmd-shift-t-ignored',
    chords: [],
    label: 'New Feature (with Shift)',
    description:
      'Intentionally not bound. The browser / Tauri webview keeps its ' +
      'reopen-closed-tab shortcut; we never surprise the user by stealing it.',
    category: 'feature',
    badge: 'intentionally-ignored',
  },
  {
    id: 'cmd-w-close-view',
    chords: [{ primary: true, shift: false, alt: false, key: 'w' }],
    label: 'Close view',
    description:
      'Close the active modal / drawer / popover, or pop one entry off the ' +
      'in-app navigation stack.',
    category: 'view',
  },

  // ── palette ──
  {
    id: 'cmd-k-command-palette',
    chords: [{ primary: true, shift: false, alt: false, key: 'k' }],
    label: 'Command Palette',
    description: 'Open the Command Palette for fuzzy navigation and actions.',
    category: 'palette',
  },
  {
    id: 'cmd-p-palette-alias',
    chords: [{ primary: true, shift: false, alt: false, key: 'p' }],
    label: 'Command Palette',
    description: 'Alias for `Cmd/Ctrl + K` for muscle-memory VS Code users.',
    category: 'palette',
    badge: 'alias',
  },

  // ── project ──
  {
    id: 'cmd-n-new-project',
    chords: [{ primary: true, shift: false, alt: false, key: 'n' }],
    label: 'New Project',
    description: 'Start a new project from scratch or an existing repo.',
    category: 'project',
  },
  ...PROJECT_NUMBER_ENTRIES,

  // ── view ──
  {
    id: 'cmd-comma-settings',
    chords: [{ primary: true, shift: false, alt: false, key: ',' }],
    label: 'Settings',
    description: 'Open the Preferences screen.',
    category: 'view',
  },
  {
    id: 'cmd-b-toggle-sidebar',
    chords: [{ primary: true, shift: false, alt: false, key: 'b' }],
    label: 'Toggle sidebar',
    description: 'Collapse or expand the project sidebar.',
    category: 'view',
  },
  {
    id: 'cmd-backtick-toggle-terminal-panel',
    chords: [{ primary: true, shift: false, alt: false, key: '`' }],
    label: 'Open Terminals view',
    description:
      'Open the full-page Terminals view. ' +
      'Sessions stay alive as you navigate around the app.',
    category: 'view',
  },
  {
    id: 'cmd-shift-backtick-new-terminal',
    chords: NEW_TERMINAL_CHORDS,
    label: 'New terminal',
    description:
      'While on the Terminals view, open the New-terminal launcher — pick a ' +
      'machine, agent, or feature checkout to launch a session. Companion to ' +
      'Cmd/Ctrl + ` (open Terminals view).',
    category: 'view',
  },
  {
    id: 'cmd-g-next-feature',
    chords: [{ primary: true, shift: false, alt: false, key: 'g' }],
    label: 'Next feature',
    description: 'Jump to the next feature in the current project.',
    category: 'feature',
  },
  {
    id: 'cmd-shift-g-previous-feature',
    chords: [{ primary: true, shift: true, alt: false, key: 'g' }],
    label: 'Previous feature',
    description: 'Jump to the previous feature in the current project.',
    category: 'feature',
  },

  // ── help / overlay ──
  {
    id: 'f1-help',
    chords: [{ primary: false, shift: false, alt: false, key: 'F1' }],
    label: 'Show this help',
    description: 'Open the keyboard + mouse shortcut reference overlay.',
    category: 'help',
  },
  {
    id: 'question-mark-help',
    chords: [{ primary: false, shift: false, alt: false, key: '?' }],
    label: 'Show this help',
    description:
      'Alias for `F1`. Note: keyboards that require Shift to type `?` ' +
      'open the help via `Shift + ?`; F1 always works.',
    category: 'help',
    badge: 'alias',
  },
  {
    id: 'escape-close-overlay',
    chords: [{ primary: false, shift: false, alt: false, key: 'Escape' }],
    label: 'Close any overlay',
    description:
      'Close the topmost modal, drawer, popover, or the help overlay. With ' +
      'nothing open it pops one entry off the in-app navigation stack.',
    category: 'view',
  },

  // ── mouse / navigation ──
  {
    id: 'mouse-xbutton1-back',
    chords: [{ primary: false, shift: false, alt: false, key: 'MouseButton3' }],
    label: 'Navigate back',
    description:
      'Press the mouse back button (XButton1) anywhere in the app to pop the ' +
      'in-app navigation stack. Suppressed while any modal is open or a text ' +
      'field is focused.',
    category: 'mouse',
  },
  {
    id: 'mouse-xbutton2-forward',
    chords: [{ primary: false, shift: false, alt: false, key: 'MouseButton4' }],
    label: 'Navigate forward',
    description:
      'Press the mouse forward button (XButton2) to advance the in-app ' +
      'navigation stack. Suppressed while any modal is open or a text field ' +
      'is focused.',
    category: 'mouse',
  },
  {
    id: 'alt-left-back',
    chords: [{ primary: false, shift: false, alt: true, key: 'ArrowLeft' }],
    label: 'Navigate back',
    description:
      'Keyboard alias for the mouse back button. Shares the same in-app ' +
      'navigation stack.',
    category: 'navigation',
  },
  {
    id: 'alt-right-forward',
    chords: [{ primary: false, shift: false, alt: true, key: 'ArrowRight' }],
    label: 'Navigate forward',
    description:
      'Keyboard alias for the mouse forward button. Shares the same in-app ' +
      'navigation stack.',
    category: 'navigation',
  },

  // ── data ──
  {
    id: 'cmd-r-reload-data',
    chords: [{ primary: true, shift: false, alt: false, key: 'r' }],
    label: 'Reload data',
    description:
      'Refresh the current view — refetches projects / features / notifications ' +
      'from the backend without restarting the app.',
    category: 'data',
  },
  {
    id: 'f11-fullscreen',
    chords: [{ primary: false, shift: false, alt: false, key: 'F11' }],
    label: 'Toggle fullscreen',
    description: 'Toggle the Tauri window between fullscreen and windowed mode.',
    category: 'data',
  },
];

export const SHORTCUT_GROUPS: readonly ShortcutGroup[] = [
  {
    id: 'navigation',
    title: 'Navigation',
    description: 'Move through the app — features, projects, history.',
    entries: SHORTCUTS.filter((entry) => entry.category === 'navigation'),
  },
  {
    id: 'feature',
    title: 'Features',
    description: 'Start and step through features inside the current project.',
    entries: SHORTCUTS.filter((entry) => entry.category === 'feature'),
  },
  {
    id: 'project',
    title: 'Projects',
    description: 'Create new projects and switch between the ones you have open.',
    entries: SHORTCUTS.filter((entry) => entry.category === 'project'),
  },
  {
    id: 'view',
    title: 'View',
    description: 'Settings, sidebar, and overlay controls.',
    entries: SHORTCUTS.filter((entry) => entry.category === 'view'),
  },
  {
    id: 'palette',
    title: 'Command Palette',
    description: 'Fuzzy launcher for every action in the app.',
    entries: SHORTCUTS.filter((entry) => entry.category === 'palette'),
  },
  {
    id: 'mouse',
    title: 'Mouse',
    description: 'Hardware buttons + keyboard aliases.',
    entries: SHORTCUTS.filter((entry) => entry.category === 'mouse'),
  },
  {
    id: 'data',
    title: 'Data & Window',
    description: 'Reload state and toggle the window.',
    entries: SHORTCUTS.filter((entry) => entry.category === 'data'),
  },
  {
    id: 'help',
    title: 'Help',
    description: 'How to summon this overlay.',
    entries: SHORTCUTS.filter((entry) => entry.category === 'help'),
  },
];

// ── Matcher ─────────────────────────────────────────────────────────────

export interface KeyboardEventLike {
  key: string;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
}

export interface MouseEventLike {
  /**
   * Mouse button index — 0 left, 1 middle, 2 right, 3 XButton1 (back),
   * 4 XButton2 (forward). Only 3 / 4 are recognised by the matcher.
   */
  button: number;
}

const ALPHA_CHORD_KEYS = new Set<string>([
  // Common KeyboardEvent.key values that are encoded as a length-1 string
  // and therefore match case-insensitively in the matcher.
  ...'abcdefghijklmnopqrstuvwxyz',
  ...'0123456789',
]);

function normaliseKey(raw: string): string {
  // Single-character alpha / digit → lower-case. Punctuation and named
  // special keys preserve their canonical form.
  if (raw.length === 1 && ALPHA_CHORD_KEYS.has(raw.toLowerCase())) {
    return raw.toLowerCase();
  }
  return raw;
}

export function matchesKeyEvent(
  event: KeyboardEventLike,
  chord: ShortcutChord,
): boolean {
  const eventKey = normaliseKey(event.key);
  const chordKey = normaliseKey(chord.key);
  if (eventKey !== chordKey) return false;

  // Platform collapse: the dispatcher accepts Cmd OR Ctrl as the "primary"
  // key because the backend is the same code path either way.
  const primary = event.metaKey || event.ctrlKey;
  if (chord.primary !== primary) return false;
  if (chord.shift !== event.shiftKey) return false;
  if (chord.alt !== event.altKey) return false;
  return true;
}

/**
 * Whether a key event landed in something the user is typing into.
 *
 * Any chord whose key is a character the user can type has to ask this before
 * consuming the event, and the cost of forgetting is invisible: bare `?` is
 * bound to the docs panel with `preventDefault`, so an unguarded dispatcher
 * eats the `?` out of whatever field has focus and opens a panel over it. The
 * two global `keydown` listeners disagreed about this — `ShortcutHelp` guarded,
 * `useKeyboardShortcuts` did not — which is audit finding F5's shape, and the
 * reason the predicate lives here rather than a third time at a call site.
 *
 * `SELECT` is deliberately absent: a select consumes printable keys to jump
 * between options, but it is not a text field, and `WorkflowBuilder` includes it
 * only because a canvas delete-key would otherwise fire from its own toolbar.
 */
export function isEditableTarget(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    // `=== true` rather than a bare read: the DOM types promise a boolean and
    // jsdom returns `undefined`, so `&&` would hand back a non-boolean through
    // a signature that says otherwise.
    (target instanceof HTMLElement && target.isContentEditable === true)
  );
}

export function matchesMouseButton(
  event: MouseEventLike,
  chord: ShortcutChord,
): boolean {
  if (chord.key !== 'MouseButton3' && chord.key !== 'MouseButton4') return false;
  // Mouse chords never coexist with keyboard modifiers in this app.
  if (chord.primary || chord.shift || chord.alt) return false;
  if (chord.key === 'MouseButton3') return event.button === 3;
  return event.button === 4;
}

/**
 * Check whether `event` matches **any** of the chords on the given entry.
 * Empty-chord entries (intentionally-ignored placeholders) never match.
 */
export function matchesEntryKeyboard(
  event: KeyboardEventLike,
  entry: ShortcutEntry,
): boolean {
  if (entry.chords.length === 0) return false;
  return entry.chords.some((chord) => matchesKeyEvent(event, chord));
}

export function matchesEntryMouse(
  event: MouseEventLike,
  entry: ShortcutEntry,
): boolean {
  if (entry.chords.length === 0) return false;
  return entry.chords.some((chord) => matchesMouseButton(event, chord));
}

// ── Formatter ───────────────────────────────────────────────────────────

export type ShortcutPlatform = 'mac' | 'other';

export interface ChordFormatOptions {
  /** Platform to render for. Defaults to `'other'`. */
  platform?: ShortcutPlatform;
}

function displayKey(raw: string, platform: ShortcutPlatform): string {
  switch (raw) {
    case ' ': return platform === 'mac' ? 'Space' : 'Space';
    case 'ArrowLeft': return platform === 'mac' ? '←' : 'Left';
    case 'ArrowRight': return platform === 'mac' ? '→' : 'Right';
    case 'ArrowUp': return platform === 'mac' ? '↑' : 'Up';
    case 'ArrowDown': return platform === 'mac' ? '↓' : 'Down';
    case 'Escape': return platform === 'mac' ? '⎋' : 'Esc';
    case 'Enter': return platform === 'mac' ? '⏎' : 'Enter';
    case 'MouseButton3': return 'XButton1 (mouse back)';
    case 'MouseButton4': return 'XButton2 (mouse forward)';
    default: {
      // Single-character alpha keys render upper-case in docs / help overlay
      // style — "Cmd + K", not "Cmd + k". Punctuation, digits, and named
      // keys already passed through unchanged in the cases above.
      if (raw.length === 1) return raw.toUpperCase();
      return raw;
    }
  }
}

function formatUniversal(chord: ShortcutChord): string {
  const parts: string[] = [];
  if (chord.primary) parts.push('Cmd/Ctrl');
  if (chord.shift) parts.push('Shift');
  if (chord.alt) parts.push('Alt');
  parts.push(displayKey(chord.key, 'other'));
  return parts.join(' + ');
}

function formatPlatform(chord: ShortcutChord, platform: ShortcutPlatform): string {
  const parts: string[] = [];
  if (chord.primary) parts.push(platform === 'mac' ? '⌘' : 'Ctrl');
  if (chord.shift) parts.push(platform === 'mac' ? '⇧' : 'Shift');
  if (chord.alt) parts.push(platform === 'mac' ? '⌥' : 'Alt');
  parts.push(displayKey(chord.key, platform));
  // Mac rhythm: glyphs joined tightly ("⌘ T"), other platforms use " + ".
  return platform === 'mac' ? parts.join('') : parts.join(' + ');
}

/**
 * Render a single chord for the help overlay.
 *   - `mode: 'universal'`  → "Cmd/Ctrl + T" (default — matches the rest
 *      of the docs / keyboard-shortcuts.md style).
 *   - `mode: 'mac'`        → "⌘ T"
 *   - `mode: 'other'`      → "Ctrl + T"
 */
export function formatChord(
  chord: ShortcutChord,
  mode: 'universal' | 'mac' | 'other' = 'universal',
): string {
  if (mode === 'universal') return formatUniversal(chord);
  return formatPlatform(chord, mode);
}

/**
 * Render every chord for an entry as a single display string. An empty
 * chord list produces "—" so the help overlay never renders an empty cell.
 */
export function formatEntryChords(
  entry: ShortcutEntry,
  mode: 'universal' | 'mac' | 'other' = 'universal',
): string {
  if (entry.chords.length === 0) return '—';
  return entry.chords.map((chord) => formatChord(chord, mode)).join('  /  ');
}

/**
 * Quick lookup. Returns the entry whose `id` matches, or `undefined`.
 * Useful for tests + deep-link style navigation ("?open=cmd-t-new-feature").
 */
export function findShortcutById(id: string): ShortcutEntry | undefined {
  return SHORTCUTS.find((entry) => entry.id === id);
}

/**
 * Ensure every id in the registry is unique. Returns a list of duplicate
 * ids; exposed so a startup-time smoke test can detect a typo before the
 * help overlay renders.
 */
export function findDuplicateShortcutIds(): readonly string[] {
  const seen = new Set<string>();
  const duplicates: string[] = [];
  for (const entry of SHORTCUTS) {
    if (seen.has(entry.id)) {
      duplicates.push(entry.id);
    } else {
      seen.add(entry.id);
    }
  }
  return duplicates;
}
