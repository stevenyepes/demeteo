// Unit tests for `src/lib/shortcuts.ts`.

import { describe, expect, it } from 'vitest';

import {
  SHORTCUTS,
  SHORTCUT_GROUPS,
  findDuplicateShortcutIds,
  findShortcutById,
  formatChord,
  formatEntryChords,
  isEditableTarget,
  matchesEntryKeyboard,
  matchesEntryMouse,
  matchesKeyEvent,
  matchesMouseButton,
  type ShortcutChord,
} from './shortcuts';

function ev(
  init: Partial<{
    key: string;
    shiftKey: boolean;
    altKey: boolean;
    metaKey: boolean;
    ctrlKey: boolean;
  }>,
): Parameters<typeof matchesKeyEvent>[0] {
  return {
    key: init.key ?? '',
    shiftKey: init.shiftKey ?? false,
    altKey: init.altKey ?? false,
    metaKey: init.metaKey ?? false,
    ctrlKey: init.ctrlKey ?? false,
  };
}

const chordTPrimary: ShortcutChord = { primary: true, shift: false, alt: false, key: 't' };
const chordShiftN: ShortcutChord = { primary: true, shift: true, alt: false, key: 'n' };
const chordF1: ShortcutChord = { primary: false, shift: false, alt: false, key: 'F1' };
const chordQuestion: ShortcutChord = { primary: false, shift: false, alt: false, key: '?' };
const chordAltLeft: ShortcutChord = { primary: false, shift: false, alt: true, key: 'ArrowLeft' };
const chordXButton1: ShortcutChord = {
  primary: false,
  shift: false,
  alt: false,
  key: 'MouseButton3',
};
const chordXButton2: ShortcutChord = {
  primary: false,
  shift: false,
  alt: false,
  key: 'MouseButton4',
};

describe('registry invariants', () => {
  it('has no duplicate ids', () => {
    expect(findDuplicateShortcutIds()).toEqual([]);
  });

  it('holds a non-trivial number of entries', () => {
    expect(SHORTCUTS.length).toBeGreaterThanOrEqual(10);
  });

  it('gives every entry a group, id, label, description and chord array', () => {
    const groupIds = new Set(SHORTCUT_GROUPS.map((group) => group.id));

    for (const entry of SHORTCUTS) {
      expect(groupIds, `entry "${entry.id}" has an unknown category`).toContain(entry.category);
      expect(entry.id).toBeTruthy();
      expect(entry.label, `entry "${entry.id}" is missing a label`).toBeTruthy();
      expect(entry.description, `entry "${entry.id}" is missing a description`).toBeTruthy();
      // Even intentionally-ignored placeholders carry an (empty) chord array.
      expect(Array.isArray(entry.chords), `entry "${entry.id}" chords must be an array`).toBe(true);
    }
  });
});

describe('the intentionally-ignored sentinel', () => {
  // The help panel surfaces Cmd/Ctrl+Shift+T as a deliberate non-binding.
  it('exists, binds nothing, and carries the badge', () => {
    const ignored = findShortcutById('cmd-shift-t-ignored');

    expect(ignored).toBeDefined();
    expect(ignored!.chords).toEqual([]);
    expect(ignored!.badge).toBe('intentionally-ignored');
  });
});

describe('matchesKeyEvent', () => {
  it('matches Cmd+T and Ctrl+T against a primary-bound chord', () => {
    expect(matchesKeyEvent(ev({ key: 't', metaKey: true }), chordTPrimary)).toBe(true);
    expect(matchesKeyEvent(ev({ key: 't', ctrlKey: true }), chordTPrimary)).toBe(true);
  });

  it('is case-insensitive on the key when no shift is held', () => {
    expect(matchesKeyEvent(ev({ key: 'T', metaKey: true, shiftKey: false }), chordTPrimary)).toBe(
      true,
    );
  });

  // `shift: false` is strict: without it Cmd+Shift+T would leak through to
  // the Cmd+T handler.
  it('rejects Cmd+Shift+T for a shift:false chord', () => {
    expect(matchesKeyEvent(ev({ key: 'T', metaKey: true, shiftKey: true }), chordTPrimary)).toBe(
      false,
    );
  });

  it('rejects a bare key against a primary-bound chord', () => {
    expect(matchesKeyEvent(ev({ key: 't' }), chordTPrimary)).toBe(false);
  });

  it('requires shift when the chord demands it', () => {
    expect(matchesKeyEvent(ev({ key: 'n', metaKey: true, shiftKey: true }), chordShiftN)).toBe(true);
    expect(matchesKeyEvent(ev({ key: 'n', metaKey: true, shiftKey: false }), chordShiftN)).toBe(
      false,
    );
  });

  it('treats primary:false chords as strict — no extra modifiers allowed', () => {
    expect(matchesKeyEvent(ev({ key: 'F1' }), chordF1)).toBe(true);
    expect(matchesKeyEvent(ev({ key: 'F1', metaKey: true }), chordF1)).toBe(false);
  });

  it('does not let the help chord steal Shift+? or Cmd+?', () => {
    expect(matchesKeyEvent(ev({ key: '?' }), chordQuestion)).toBe(true);
    expect(matchesKeyEvent(ev({ key: '?', shiftKey: true }), chordQuestion)).toBe(false);
    expect(matchesKeyEvent(ev({ key: '?', metaKey: true }), chordQuestion)).toBe(false);
  });

  it('matches Alt+ArrowLeft only with Alt and nothing else', () => {
    expect(matchesKeyEvent(ev({ key: 'ArrowLeft', altKey: true }), chordAltLeft)).toBe(true);
    expect(matchesKeyEvent(ev({ key: 'ArrowLeft' }), chordAltLeft)).toBe(false);
    expect(matchesKeyEvent(ev({ key: 'ArrowLeft', altKey: true, metaKey: true }), chordAltLeft)).toBe(
      false,
    );
  });
});

describe('matchesMouseButton', () => {
  it('maps buttons 3 and 4 to their chords', () => {
    expect(matchesMouseButton({ button: 3 }, chordXButton1)).toBe(true);
    expect(matchesMouseButton({ button: 4 }, chordXButton2)).toBe(true);
  });

  it('does not confuse buttons with each other or with a left click', () => {
    expect(matchesMouseButton({ button: 0 }, chordXButton1)).toBe(false);
    expect(matchesMouseButton({ button: 3 }, chordXButton2)).toBe(false);
  });

  // Catches platform leakage where a key listener fires on a mouse event.
  it('rejects a mouse chord that also demands modifiers', () => {
    expect(
      matchesMouseButton(
        { button: 3 },
        { primary: false, shift: true, alt: false, key: 'MouseButton3' },
      ),
    ).toBe(false);
  });
});

describe('matchesEntryKeyboard / matchesEntryMouse', () => {
  it('matches Cmd+T against the new-feature entry', () => {
    expect(matchesEntryKeyboard(ev({ key: 't', metaKey: true }), findShortcutById('cmd-t-new-feature')!)).toBe(
      true,
    );
  });

  it('matches Cmd+Shift+` against the new-terminal entry', () => {
    expect(
      matchesEntryKeyboard(
        ev({ key: '`', metaKey: true, shiftKey: true }),
        findShortcutById('cmd-shift-backtick-new-terminal')!,
      ),
    ).toBe(true);
  });

  it('never matches the empty-chord sentinel', () => {
    expect(
      matchesEntryKeyboard(
        ev({ key: 'T', metaKey: true, shiftKey: true }),
        findShortcutById('cmd-shift-t-ignored')!,
      ),
    ).toBe(false);
  });

  it('routes mouse button 3 to back, not forward', () => {
    expect(matchesEntryMouse({ button: 3 }, findShortcutById('mouse-xbutton1-back')!)).toBe(true);
    expect(matchesEntryMouse({ button: 3 }, findShortcutById('mouse-xbutton2-forward')!)).toBe(false);
  });

  // Spec §7 Q4: `Cmd/Ctrl + \`` toggles the terminal panel.
  it('matches Cmd+` and Ctrl+` against the terminal-panel entry', () => {
    const entry = findShortcutById('cmd-backtick-toggle-terminal-panel');
    expect(entry).toBeDefined();
    expect(matchesEntryKeyboard(ev({ key: '`', metaKey: true }), entry!)).toBe(true);
    expect(matchesEntryKeyboard(ev({ key: '`', ctrlKey: true }), entry!)).toBe(true);
  });

  it('rejects a bare backtick against the terminal-panel entry (primary required)', () => {
    const entry = findShortcutById('cmd-backtick-toggle-terminal-panel')!;
    expect(matchesEntryKeyboard(ev({ key: '`' }), entry)).toBe(false);
  });
});

describe('formatChord', () => {
  const k: ShortcutChord = { primary: true, shift: false, alt: false, key: 'k' };

  it('renders the primary modifier per platform', () => {
    expect(formatChord(k)).toBe('Cmd/Ctrl + K');
    expect(formatChord(k, 'mac')).toBe('⌘K');
    expect(formatChord(k, 'other')).toBe('Ctrl + K');
  });

  it('includes shift when present', () => {
    expect(formatChord({ primary: true, shift: true, alt: false, key: 'n' })).toBe(
      'Cmd/Ctrl + Shift + N',
    );
  });

  it('renders arrows as glyphs on mac and words elsewhere', () => {
    const altLeft: ShortcutChord = { primary: false, shift: false, alt: true, key: 'ArrowLeft' };

    expect(formatChord(altLeft, 'mac')).toBe('⌥←');
    expect(formatChord(altLeft, 'other')).toBe('Alt + Left');
  });

  it('passes function keys through and labels mouse chords with the hardware name', () => {
    expect(formatChord({ primary: false, shift: false, alt: false, key: 'F1' })).toBe('F1');
    expect(formatChord({ primary: false, shift: false, alt: false, key: 'MouseButton3' })).toBe(
      'XButton1 (mouse back)',
    );
  });

  // Matches the help overlay's "omit the Cmd/Ctrl +" rule for bare keys.
  it('renders a bare key with no modifier prefix', () => {
    expect(formatChord({ primary: false, shift: false, alt: false, key: ',' })).toBe(',');
  });
});

describe('formatEntryChords', () => {
  // Keeps the help overlay from rendering a blank cell.
  it('renders an empty chord list as an em-dash', () => {
    expect(formatEntryChords(findShortcutById('cmd-shift-t-ignored')!)).toBe('—');
  });

  it('renders the key label for a bound entry', () => {
    expect(formatEntryChords(findShortcutById('cmd-t-new-feature')!)).toContain('T');
  });
});

// Any new shortcut the team adds must land in this table. Registering a
// shortcut anywhere else in the app without listing it here fails the build.
const EXPECTED_CHORDS: { id: string; key: string; primary: boolean; shift: boolean; alt: boolean }[] = [
  { id: 'cmd-t-new-feature', key: 't', primary: true, shift: false, alt: false },
  { id: 'cmd-shift-n-new-feature-alias', key: 'n', primary: true, shift: true, alt: false },
  { id: 'cmd-shift-backtick-new-terminal', key: '`', primary: true, shift: true, alt: false },
  { id: 'cmd-w-close-view', key: 'w', primary: true, shift: false, alt: false },
  { id: 'cmd-k-command-palette', key: 'k', primary: true, shift: false, alt: false },
  { id: 'cmd-p-palette-alias', key: 'p', primary: true, shift: false, alt: false },
  { id: 'cmd-n-new-project', key: 'n', primary: true, shift: false, alt: false },
  { id: 'cmd-comma-settings', key: ',', primary: true, shift: false, alt: false },
  { id: 'cmd-b-toggle-sidebar', key: 'b', primary: true, shift: false, alt: false },
  { id: 'cmd-backtick-toggle-terminal-panel', key: '`', primary: true, shift: false, alt: false },
  { id: 'cmd-g-next-feature', key: 'g', primary: true, shift: false, alt: false },
  { id: 'cmd-shift-g-previous-feature', key: 'g', primary: true, shift: true, alt: false },
  { id: 'j-next-step', key: 'j', primary: false, shift: false, alt: false },
  { id: 'k-previous-step', key: 'k', primary: false, shift: false, alt: false },
  { id: 'enter-focus-inspector', key: 'Enter', primary: false, shift: false, alt: false },
  { id: 'g-graph-view', key: 'g', primary: false, shift: false, alt: false },
  { id: 't-timeline-view', key: 't', primary: false, shift: false, alt: false },
  { id: 'f1-help', key: 'F1', primary: false, shift: false, alt: false },
  { id: 'question-mark-help', key: '?', primary: false, shift: false, alt: false },
  { id: 'escape-close-overlay', key: 'Escape', primary: false, shift: false, alt: false },
  { id: 'mouse-xbutton1-back', key: 'MouseButton3', primary: false, shift: false, alt: false },
  { id: 'mouse-xbutton2-forward', key: 'MouseButton4', primary: false, shift: false, alt: false },
  { id: 'alt-left-back', key: 'ArrowLeft', primary: false, shift: false, alt: true },
  { id: 'alt-right-forward', key: 'ArrowRight', primary: false, shift: false, alt: true },
  { id: 'cmd-r-reload-data', key: 'r', primary: true, shift: false, alt: false },
  { id: 'f11-fullscreen', key: 'F11', primary: false, shift: false, alt: false },
];

describe('mandatory entry coverage', () => {
  it.each(EXPECTED_CHORDS)('$id binds the expected chord', ({ id, ...expected }) => {
    const entry = findShortcutById(id);

    expect(entry, `missing entry ${id}`).toBeDefined();
    expect(entry!.chords[0]).toMatchObject(expected);
  });

  it.each([1, 2, 3, 4, 5, 6, 7, 8, 9])('registers the project-switch shortcut for %i', (n) => {
    expect(findShortcutById(`cmd-${n}-switch-project-${n}`)).toBeDefined();
  });

  it('registers the sentinel entry that binds nothing', () => {
    expect(findShortcutById('cmd-shift-t-ignored')).toBeDefined();
  });
});

describe('the run-view group', () => {
  const RUN_ENTRY_IDS = [
    'j-next-step',
    'k-previous-step',
    'enter-focus-inspector',
    'g-graph-view',
    't-timeline-view',
  ];

  it('carries all five run entries', () => {
    const group = SHORTCUT_GROUPS.find((candidate) => candidate.id === 'run');

    expect(group).toBeDefined();
    expect(group!.entries.map((entry) => entry.id)).toEqual(RUN_ENTRY_IDS);
  });

  // The overlay skips empty groups, and these keys are the registry's only
  // non-global ones — a group with no copy would advertise them as app-wide.
  it('tells the reader the keys are scoped to the run view', () => {
    const group = SHORTCUT_GROUPS.find((candidate) => candidate.id === 'run')!;

    expect(group.description).toBeTruthy();
    expect(group.description!.toLowerCase()).toContain('run view');
  });

  it('renders in a fixed slot, right after Features', () => {
    const ids = SHORTCUT_GROUPS.map((group) => group.id);

    expect(ids.indexOf('run')).toBe(ids.indexOf('feature') + 1);
  });

  it.each(RUN_ENTRY_IDS)('%s fires on its bare key with no modifier held', (id) => {
    const entry = findShortcutById(id)!;
    const { key } = entry.chords[0];

    expect(matchesEntryKeyboard(ev({ key }), entry)).toBe(true);
    expect(matchesEntryKeyboard(ev({ key, metaKey: true }), entry)).toBe(false);
    expect(matchesEntryKeyboard(ev({ key, ctrlKey: true }), entry)).toBe(false);
    expect(matchesEntryKeyboard(ev({ key, shiftKey: true }), entry)).toBe(false);
    expect(matchesEntryKeyboard(ev({ key, altKey: true }), entry)).toBe(false);
  });
});

/**
 * Bare `g`/`t` and `Cmd/Ctrl + G`/`Cmd/Ctrl + T` share a key and mean four
 * different things. Nothing separates them but `matchesKeyEvent` comparing
 * `primary` as an exact boolean, so this pins that comparison: a matcher that
 * grows a "don't care" modifier mode makes every pair below fire twice.
 */
describe('bare run-view keys versus their Cmd/Ctrl twins', () => {
  it('keeps bare g out of the next-feature entry and Cmd+G out of the graph entry', () => {
    const nextFeature = findShortcutById('cmd-g-next-feature')!;
    const graphView = findShortcutById('g-graph-view')!;

    expect(matchesEntryKeyboard(ev({ key: 'g' }), nextFeature)).toBe(false);
    expect(matchesEntryKeyboard(ev({ key: 'g', metaKey: true }), graphView)).toBe(false);
    expect(matchesEntryKeyboard(ev({ key: 'g', ctrlKey: true }), graphView)).toBe(false);

    expect(matchesEntryKeyboard(ev({ key: 'g' }), graphView)).toBe(true);
    expect(matchesEntryKeyboard(ev({ key: 'g', metaKey: true }), nextFeature)).toBe(true);
  });

  it('keeps bare t out of the new-feature entry and Cmd+T out of the timeline entry', () => {
    const newFeature = findShortcutById('cmd-t-new-feature')!;
    const timelineView = findShortcutById('t-timeline-view')!;

    expect(matchesEntryKeyboard(ev({ key: 't' }), newFeature)).toBe(false);
    expect(matchesEntryKeyboard(ev({ key: 't', metaKey: true }), timelineView)).toBe(false);
    expect(matchesEntryKeyboard(ev({ key: 't', ctrlKey: true }), timelineView)).toBe(false);

    expect(matchesEntryKeyboard(ev({ key: 't' }), timelineView)).toBe(true);
    expect(matchesEntryKeyboard(ev({ key: 't', metaKey: true }), newFeature)).toBe(true);
  });

  // Cmd+Shift+G is a third meaning for the same key; the run entries must not
  // reach it either.
  it('keeps bare g out of the previous-feature entry', () => {
    expect(
      matchesEntryKeyboard(ev({ key: 'g' }), findShortcutById('cmd-shift-g-previous-feature')!),
    ).toBe(false);
  });
});

describe('group coverage', () => {
  it.each([
    'navigation',
    'feature',
    'run',
    'project',
    'view',
    'palette',
    'mouse',
    'data',
    'help',
  ])('defines the "%s" group', (id) => {
    expect(SHORTCUT_GROUPS.some((group) => group.id === id)).toBe(true);
  });
});

/**
 * Bare `?` is bound to the docs panel with `preventDefault`, so a dispatcher
 * that skips this guard eats the character out of whatever field has focus.
 * Both global `keydown` listeners route through this predicate now; they used
 * to disagree, which is audit F5's shape.
 */
describe('isEditableTarget', () => {
  it('claims the fields a user types into', () => {
    expect(isEditableTarget(document.createElement('input'))).toBe(true);
    expect(isEditableTarget(document.createElement('textarea'))).toBe(true);

    const rich = document.createElement('div');
    rich.contentEditable = 'true';
    // jsdom does not derive `isContentEditable` from the attribute.
    Object.defineProperty(rich, 'isContentEditable', { value: true });
    expect(isEditableTarget(rich)).toBe(true);
  });

  it('leaves everything else to the shortcut dispatchers', () => {
    expect(isEditableTarget(document.createElement('div'))).toBe(false);
    expect(isEditableTarget(document.createElement('button'))).toBe(false);
    expect(isEditableTarget(null)).toBe(false);
  });

  /** A select consumes printable keys to jump between options but holds no
   *  text, so a shortcut is right to fire over it. `WorkflowBuilder` counts it
   *  as editable for a different reason — a canvas Delete key — and that
   *  difference is deliberate, not drift to be reconciled here. */
  it('does not claim a select', () => {
    expect(isEditableTarget(document.createElement('select'))).toBe(false);
  });
});
