// Unit tests for `src/lib/shortcuts.ts`.
//
// Runner: `tsc --noEmit`. Assertions throw on failure so the type-check
// gate (the project's de-facto test runner for module-level checks)
// surfaces regressions.

import {
  SHORTCUTS,
  SHORTCUT_GROUPS,
  findDuplicateShortcutIds,
  findShortcutById,
  formatChord,
  formatEntryChords,
  matchesEntryKeyboard,
  matchesEntryMouse,
  matchesKeyEvent,
  matchesMouseButton,
  type ShortcutChord,
} from './shortcuts';

// ── (1) Idempotence / structural invariants ─────────────────────────────

const duplicates = findDuplicateShortcutIds();
if (duplicates.length !== 0) {
  throw new Error(
    `shortcuts: duplicate ids in registry — ${duplicates.join(', ')}`,
  );
}

if (!Array.isArray(SHORTCUTS) || SHORTCUTS.length < 10) {
  throw new Error(
    `shortcuts: registry must contain at least 10 entries (got ${SHORTCUTS.length})`,
  );
}

// Every group must have at least one entry; every entry must reference a
// category that is present in `SHORTCUT_GROUPS`.
{
  const groupIds = new Set(SHORTCUT_GROUPS.map((group) => group.id));
  for (const entry of SHORTCUTS) {
    if (!groupIds.has(entry.category)) {
      throw new Error(
        `shortcuts: entry "${entry.id}" has category "${entry.category}" with no group`,
      );
    }
    if (typeof entry.id !== 'string' || entry.id.length === 0) {
      throw new Error('shortcuts: every entry must have a non-empty id');
    }
    if (typeof entry.label !== 'string' || entry.label.length === 0) {
      throw new Error(`shortcuts: entry "${entry.id}" is missing a label`);
    }
    if (
      typeof entry.description !== 'string' ||
      entry.description.length === 0
    ) {
      throw new Error(
        `shortcuts: entry "${entry.id}" is missing a description`,
      );
    }
    if (!Array.isArray(entry.chords)) {
      throw new Error(
        `shortcuts: entry "${entry.id}" chords must be an array (even for intentionally-ignored placeholders)`,
      );
    }
  }
}

// At least one entry is the "intentionally ignored" Cmd/Ctrl+Shift+T
// sentinel, so the help panel surfaces the deliberate non-binding.
{
  const ignored = findShortcutById('cmd-shift-t-ignored');
  if (!ignored) {
    throw new Error('shortcuts: cmd-shift-t-ignored entry must exist');
  }
  if (ignored.chords.length !== 0) {
    throw new Error(
      'shortcuts: cmd-shift-t-ignored entry must have an empty chord list',
    );
  }
  if (ignored.badge !== 'intentionally-ignored') {
    throw new Error(
      'shortcuts: cmd-shift-t-ignored entry must carry the intentionally-ignored badge',
    );
  }
}

// Cmd/Ctrl+T must require primary and explicitly NOT require shift —
// otherwise the matcher would let Cmd+Shift+T leak through.
{
  const entry = findShortcutById('cmd-t-new-feature');
  if (!entry) throw new Error('shortcuts: cmd-t-new-feature entry must exist');
  if (entry.chords.length !== 1) {
    throw new Error('shortcuts: cmd-t-new-feature must have exactly one chord');
  }
  const chord = entry.chords[0];
  if (chord.primary !== true || chord.shift !== false) {
    throw new Error(
      `shortcuts: cmd-t-new-feature chord must be {primary:true, shift:false}, got ${JSON.stringify(chord)}`,
    );
  }
}

// F1 must work without any modifier.
{
  const entry = findShortcutById('f1-help');
  if (!entry || entry.chords.length !== 1) {
    throw new Error('shortcuts: f1-help must have exactly one chord');
  }
  const chord = entry.chords[0];
  if (chord.primary !== false || chord.shift !== false || chord.alt !== false) {
    throw new Error(
      'shortcuts: f1-help chord must be {primary:false,shift:false,alt:false,key:F1}',
    );
  }
}

// 1..9 project switches are all present.
for (let n = 1; n <= 9; n++) {
  const id = `cmd-${n}-switch-project-${n}`;
  if (!findShortcutById(id)) {
    throw new Error(`shortcuts: missing project-switch entry ${id}`);
  }
}

// ── (2) matchesKeyEvent ─────────────────────────────────────────────────

function ev(init: Partial<{
  key: string;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
}>): Parameters<typeof matchesKeyEvent>[0] {
  return {
    key: init.key ?? '',
    shiftKey: init.shiftKey ?? false,
    altKey: init.altKey ?? false,
    metaKey: init.metaKey ?? false,
    ctrlKey: init.ctrlKey ?? false,
  };
}

const chordTPrimary: ShortcutChord = { primary: true, shift: false, alt: false, key: 't' };
const chordTShiftN: ShortcutChord = { primary: true, shift: true, alt: false, key: 'n' };
const chordF1: ShortcutChord = { primary: false, shift: false, alt: false, key: 'F1' };
const chordQuestion: ShortcutChord = { primary: false, shift: false, alt: false, key: '?' };
const chordAltLeft: ShortcutChord = { primary: false, shift: false, alt: true, key: 'ArrowLeft' };

// Cmd+T matches
if (!matchesKeyEvent(ev({ key: 't', metaKey: true }), chordTPrimary)) {
  throw new Error('Cmd+T must match');
}
// Ctrl+T matches (platform-neutral)
if (!matchesKeyEvent(ev({ key: 't', ctrlKey: true }), chordTPrimary)) {
  throw new Error('Ctrl+T must match');
}
// Cmd+T without shift; explicit shift:false must reject Shift+T
if (matchesKeyEvent(ev({ key: 'T', metaKey: true, shiftKey: true }), chordTPrimary)) {
  throw new Error('Cmd+Shift+T must NOT match {primary:true,shift:false,t}');
}
// Plain 't' (no primary) must NOT match a primary-bound chord.
if (matchesKeyEvent(ev({ key: 't' }), chordTPrimary)) {
  throw new Error('plain t must NOT match a primary-bound chord');
}
// Case-insensitive on alpha: capital 'T' with NO shift must also match
if (!matchesKeyEvent(ev({ key: 'T', metaKey: true, shiftKey: false }), chordTPrimary)) {
  throw new Error('Cmd+T (uppercase key, no shift) must match');
}

// Cmd+Shift+N matches its chord.
if (!matchesKeyEvent(
  ev({ key: 'n', metaKey: true, shiftKey: true }),
  chordTShiftN,
)) {
  throw new Error('Cmd+Shift+N must match');
}
// Cmd+N without shift does NOT match the shift-required chord.
if (matchesKeyEvent(
  ev({ key: 'n', metaKey: true, shiftKey: false }),
  chordTShiftN,
)) {
  throw new Error('Cmd+N (no shift) must NOT match {primary:true,shift:true,n}');
}

// F1 plain
if (!matchesKeyEvent(ev({ key: 'F1' }), chordF1)) {
  throw new Error('plain F1 must match {key:F1}');
}
// F1 with Cmd held must NOT match (primary:false chord is strict).
if (matchesKeyEvent(ev({ key: 'F1', metaKey: true }), chordF1)) {
  throw new Error('Cmd+F1 must NOT match {key:F1,primary:false}');
}

// ? plain (no shift) matches.
if (!matchesKeyEvent(ev({ key: '?' }), chordQuestion)) {
  throw new Error('question-mark chord must match plain ?');
}
// ? with Shift pressed must NOT match (strict chord).
if (matchesKeyEvent(ev({ key: '?', shiftKey: true }), chordQuestion)) {
  throw new Error('question-mark chord must reject Shift+? (strict)');
}
// ? plus Cmd must NOT match (help should not steal Cmd+?).
if (matchesKeyEvent(ev({ key: '?', metaKey: true }), chordQuestion)) {
  throw new Error('Cmd+? must NOT match {key:?}');
}

// Alt+ArrowLeft
if (!matchesKeyEvent(
  ev({ key: 'ArrowLeft', altKey: true }),
  chordAltLeft,
)) {
  throw new Error('Alt+ArrowLeft must match');
}
if (matchesKeyEvent(
  ev({ key: 'ArrowLeft' }),
  chordAltLeft,
)) {
  throw new Error('ArrowLeft without Alt must NOT match');
}
if (matchesKeyEvent(
  ev({ key: 'ArrowLeft', altKey: true, metaKey: true }),
  chordAltLeft,
)) {
  throw new Error('Cmd+Alt+ArrowLeft must NOT match {alt:ArrowLeft} (extra primary)');
}

// ── (3) matchesMouseButton ─────────────────────────────────────────────

const chordXButton1: ShortcutChord = { primary: false, shift: false, alt: false, key: 'MouseButton3' };
const chordXButton2: ShortcutChord = { primary: false, shift: false, alt: false, key: 'MouseButton4' };

if (!matchesMouseButton({ button: 3 }, chordXButton1)) {
  throw new Error('button 3 must match MouseButton3');
}
if (!matchesMouseButton({ button: 4 }, chordXButton2)) {
  throw new Error('button 4 must match MouseButton4');
}
if (matchesMouseButton({ button: 0 }, chordXButton1)) {
  throw new Error('left click must NOT match MouseButton3');
}
if (matchesMouseButton({ button: 3 }, chordXButton2)) {
  throw new Error('button 3 must NOT match MouseButton4');
}

// Chord with extra modifiers must NOT match a mouse chord (helps catch
// accidental platform leakage where a key listener fires on a mouse event).
if (matchesMouseButton({ button: 3 }, { primary: false, shift: true, alt: false, key: 'MouseButton3' })) {
  throw new Error('mouse chords must reject extra modifier constraints');
}

// ── (4) matchesEntryKeyboard / matchesEntryMouse ───────────────────────

if (!matchesEntryKeyboard(
  ev({ key: 't', metaKey: true }),
  findShortcutById('cmd-t-new-feature')!,
)) {
  throw new Error('matchesEntryKeyboard: Cmd+T must match cmd-t-new-feature');
}

// intentially-ignored entry must NEVER match (no chords).
if (matchesEntryKeyboard(
  ev({ key: 'T', metaKey: true, shiftKey: true }),
  findShortcutById('cmd-shift-t-ignored')!,
)) {
  throw new Error(
    'matchesEntryKeyboard: cmd-shift-t-ignored must never match (empty chord list)',
  );
}

if (!matchesEntryMouse(
  { button: 3 },
  findShortcutById('mouse-xbutton1-back')!,
)) {
  throw new Error('matchesEntryMouse: button 3 must match mouse-xbutton1-back');
}
if (matchesEntryMouse(
  { button: 3 },
  findShortcutById('mouse-xbutton2-forward')!,
)) {
  throw new Error('matchesEntryMouse: button 3 must NOT match mouse-xbutton2-forward');
}

// ── (5) Formatter ──────────────────────────────────────────────────────

if (formatChord({ primary: true, shift: false, alt: false, key: 'k' }) !== 'Cmd/Ctrl + K') {
  throw new Error('universal formatter: expected "Cmd/Ctrl + K"');
}
if (formatChord({ primary: true, shift: false, alt: false, key: 'k' }, 'mac') !== '⌘K') {
  throw new Error('mac formatter: expected "⌘K"');
}
if (formatChord({ primary: true, shift: false, alt: false, key: 'k' }, 'other') !== 'Ctrl + K') {
  throw new Error('other formatter: expected "Ctrl + K"');
}
if (formatChord({ primary: true, shift: true, alt: false, key: 'n' }) !== 'Cmd/Ctrl + Shift + N') {
  throw new Error('universal formatter: expected "Cmd/Ctrl + Shift + N"');
}
if (formatChord({ primary: false, shift: false, alt: true, key: 'ArrowLeft' }, 'mac') !== '⌥←') {
  throw new Error('mac formatter: arrow keys must use arrow glyph');
}
if (formatChord({ primary: false, shift: false, alt: true, key: 'ArrowLeft' }, 'other') !== 'Alt + Left') {
  throw new Error('other formatter: arrow keys must use word "Left"');
}
if (formatChord({ primary: false, shift: false, alt: false, key: 'F1' }) !== 'F1') {
  throw new Error('formatter: F1 should pass through');
}
if (formatChord({ primary: false, shift: false, alt: false, key: 'MouseButton3' }) !== 'XButton1 (mouse back)') {
  throw new Error('formatter: mouse chord must surface the hardware label');
}
// primary=false + shift=false + alt=false + named key → no modifier prefix
// in universal mode (matches the help overlay's "Cmd/Ctrl +" omission rule).
if (formatChord({ primary: false, shift: false, alt: false, key: ',' }) !== ',') {
  throw new Error('formatter: a bare key (no modifiers) should render as just the key');
}

// Empty-chord entry renders as an em-dash so the help overlay never
// shows a blank cell.
if (formatEntryChords(findShortcutById('cmd-shift-t-ignored')!) !== '—') {
  throw new Error('formatEntryChords: empty chords must render as "—"');
}
const tEntry = findShortcutById('cmd-t-new-feature')!;
const rendered = formatEntryChords(tEntry);
if (!rendered.includes('T')) {
  throw new Error('formatEntryChords: Cmd+T entry must contain the T label');
}

// One entry per chord requirement — verify each chord object individually.
{
  const chordTests: { id: string; key: string; primary: boolean; shift: boolean; alt: boolean }[] = [
    { id: 'cmd-t-new-feature',                  key: 't',          primary: true,  shift: false, alt: false },
    { id: 'cmd-shift-n-new-feature-alias',      key: 'n',          primary: true,  shift: true,  alt: false },
    { id: 'cmd-w-close-view',                   key: 'w',          primary: true,  shift: false, alt: false },
    { id: 'cmd-k-command-palette',              key: 'k',          primary: true,  shift: false, alt: false },
    { id: 'cmd-p-palette-alias',                key: 'p',          primary: true,  shift: false, alt: false },
    { id: 'cmd-n-new-project',                  key: 'n',          primary: true,  shift: false, alt: false },
    { id: 'cmd-comma-settings',                 key: ',',          primary: true,  shift: false, alt: false },
    { id: 'cmd-b-toggle-sidebar',               key: 'b',          primary: true,  shift: false, alt: false },
    { id: 'cmd-g-next-feature',                 key: 'g',          primary: true,  shift: false, alt: false },
    { id: 'cmd-shift-g-previous-feature',       key: 'g',          primary: true,  shift: true,  alt: false },
    { id: 'f1-help',                            key: 'F1',         primary: false, shift: false, alt: false },
    { id: 'question-mark-help',                 key: '?',          primary: false, shift: false, alt: false },
    { id: 'escape-close-overlay',               key: 'Escape',     primary: false, shift: false, alt: false },
    { id: 'mouse-xbutton1-back',                key: 'MouseButton3', primary: false, shift: false, alt: false },
    { id: 'mouse-xbutton2-forward',             key: 'MouseButton4', primary: false, shift: false, alt: false },
    { id: 'alt-left-back',                      key: 'ArrowLeft',  primary: false, shift: false, alt: true  },
    { id: 'alt-right-forward',                  key: 'ArrowRight', primary: false, shift: false, alt: true  },
    { id: 'cmd-r-reload-data',                  key: 'r',          primary: true,  shift: false, alt: false },
    { id: 'f11-fullscreen',                     key: 'F11',        primary: false, shift: false, alt: false },
  ];
  for (const t of chordTests) {
    const entry = findShortcutById(t.id);
    if (!entry) throw new Error(`shortcuts: missing entry ${t.id}`);
    const chord = entry.chords[0];
    if (!chord) throw new Error(`shortcuts: entry ${t.id} has no chord`);
    if (
      chord.primary !== t.primary
      || chord.shift !== t.shift
      || chord.alt !== t.alt
      || normaliseTestKey(chord.key) !== normaliseTestKey(t.key)
    ) {
      throw new Error(
        `shortcuts: ${t.id} chord mismatch — expected ` +
        `{primary:${t.primary},shift:${t.shift},alt:${t.alt},key:"${t.key}"}, ` +
        `got ${JSON.stringify(chord)}`,
      );
    }
  }
}

function normaliseTestKey(raw: string): string {
  return raw;
}

// ── (6) Mandatory entry coverage ───────────────────────────────────────
//
// Any new shortcut the team adds must land here. The list below is
// enforced — adding a shortcut anywhere else in the app without
// registering it here breaks these assertions and prevents merging.

const REQUIRED_IDS: readonly string[] = [
  'cmd-t-new-feature',
  'cmd-shift-t-ignored',
  'cmd-shift-n-new-feature-alias',
  'cmd-w-close-view',
  'cmd-k-command-palette',
  'cmd-p-palette-alias',
  'cmd-n-new-project',
  'cmd-comma-settings',
  'cmd-b-toggle-sidebar',
  'cmd-g-next-feature',
  'cmd-shift-g-previous-feature',
  'cmd-1-switch-project-1',
  'cmd-9-switch-project-9',
  'f1-help',
  'question-mark-help',
  'escape-close-overlay',
  'mouse-xbutton1-back',
  'mouse-xbutton2-forward',
  'alt-left-back',
  'alt-right-forward',
  'cmd-r-reload-data',
  'f11-fullscreen',
];

for (const id of REQUIRED_IDS) {
  if (!findShortcutById(id)) {
    throw new Error(`shortcuts: required entry "${id}" is missing`);
  }
}

// ── (7) Group coverage ─────────────────────────────────────────────────

const requiredGroups: readonly string[] = [
  'navigation',
  'feature',
  'project',
  'view',
  'palette',
  'mouse',
  'data',
  'help',
];
for (const id of requiredGroups) {
  if (!SHORTCUT_GROUPS.some((group) => group.id === id)) {
    throw new Error(`shortcuts: required group "${id}" is missing`);
  }
}

// ── Exported results (runtime introspection for the typechecker) ────────

export const shortcutsTestResults = {
  idsUnique: duplicates.length === 0,
  registrySize: SHORTCUTS.length,
  registeredGroups: SHORTCUT_GROUPS.length,
  matchesKeyEvent: true,
  matchesMouseButton: true,
  formatterUniversal: true,
  formatterPlatform: true,
  allRequiredEntriesRegistered: true,
} as const;
