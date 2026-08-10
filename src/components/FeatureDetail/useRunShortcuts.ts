import { useEffect, useRef, type RefObject } from 'react';

import {
  findShortcutById,
  isEditableTarget,
  matchesEntryKeyboard,
  type ShortcutEntry,
} from '../../lib/shortcuts';
import { adjacentStepSelection } from '../../lib/stepNavigation';
import type { StepExecution } from '../../types';
import type { RunViewMode } from '../RunViewToggle';

/**
 * The five single-key chords that live only on a feature's run view
 * (UI_REDESIGN_PLAN §3.6).
 *
 * **Nothing here spells a key.** The registry owns which chord fires which
 * action and this hook owns only what each action does; the alternative — an
 * inline `event.key === 'j'` — is the fourth disagreeing source of truth audit
 * F5 counted three of, and §3.6 rates it worse than shipping no shortcuts at
 * all. `useKeyboardShortcuts` stays the global dispatcher and never sees these;
 * why bare `g`/`t` do not shadow their Cmd/Ctrl twins is recorded beside the
 * entries in `shortcuts.ts`.
 */
const RUN_SHORTCUT_IDS = [
  'j-next-step',
  'k-previous-step',
  'enter-focus-inspector',
  'g-graph-view',
  't-timeline-view',
] as const;

export type RunShortcutId = (typeof RUN_SHORTCUT_IDS)[number];

/** Exported for the one assertion worth making about the lookup: an id renamed
 *  in the registry drops out of this list silently, leaving a key that fires
 *  nothing and a help overlay that still advertises it. */
export const RUN_SHORTCUT_ENTRIES: readonly (readonly [RunShortcutId, ShortcutEntry])[] =
  RUN_SHORTCUT_IDS.flatMap((id) => {
    const entry = findShortcutById(id);
    return entry ? [[id, entry] as const] : [];
  });

export interface RunShortcutsInput {
  /** Off while an overlay owns the keyboard. Most of them are not mounted by the
   *  run view at all — the palette, the docs panel and the start-feature modal
   *  are `App.tsx`'s siblings of it — so this cannot be derived here, and none
   *  of them moves focus, which is what makes the editable-target guard alone
   *  insufficient. `FeatureDetail` composes the full set. */
  enabled: boolean;
  steps: StepExecution[];
  selectedStepId: string | null;
  selectStep: (stepId: string) => void;
  /** The element wrapping the inspector pane, which `Enter` focuses into. */
  inspectorRef: RefObject<HTMLElement | null>;
  /** False for a run with no pinned definition, where `RunViewToggle` is not
   *  rendered either. `g` and `t` are then not bound at all: firing them would
   *  change nothing on screen, but `runViewModePref` is global, so the next run
   *  that *does* have a graph would open in a mode chosen on a surface that
   *  gave no sign of having changed. */
  canShowGraph: boolean;
  setViewMode: (mode: RunViewMode) => void;
}

export function useRunShortcuts(input: RunShortcutsInput): void {
  const ref = useRef(input);
  ref.current = input;

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const run = ref.current;
      if (!run.enabled) return;
      // A run surface may bind one of these keys itself: the graph canvas
      // claims Enter to activate the selected node. React delegates at the root
      // (`main.tsx`), so that handler has already run and already called
      // `preventDefault` by the time a window listener sees the event. Without
      // this, Enter on the graph toggles the node's selection off and then
      // focuses the inspector it just emptied.
      if (event.defaultPrevented) return;
      if (typingTarget(event.target)) return;

      const id = runShortcutFor(event);
      if (id === null) return;
      if (!fire(id, run, event.target)) return;
      event.preventDefault();
    };

    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);
}

/**
 * Move focus into the inspector, and report whether it went.
 *
 * The target is the pane's roving-tabindex entry point — `TabBar` leaves
 * exactly one tab at `tabindex="0"` (`ui/rovingIndex.ts`) — so Enter lands where
 * a Tab key would have, and it stays there: nothing in `TabBar` calls `focus`
 * outside its own keydown handler, so the 1 Hz reload does not take it back.
 *
 * The pane itself is the fallback, and the reason the run view wraps it in a
 * `tabIndex={-1}` element: an empty inspector has no tab strip and no focusable
 * child at all, so without one Enter would be a key that sometimes does nothing
 * and never says which time.
 */
export function focusInspectorPane(pane: HTMLElement | null): boolean {
  if (!pane) return false;
  const entry = pane.querySelector<HTMLElement>('[role="tab"][tabindex="0"]') ?? pane;
  entry.focus();
  return pane.contains(pane.ownerDocument.activeElement);
}

function runShortcutFor(event: KeyboardEvent): RunShortcutId | null {
  for (const [id, entry] of RUN_SHORTCUT_ENTRIES) {
    if (matchesEntryKeyboard(event, entry)) return id;
  }
  return null;
}

function fire(id: RunShortcutId, run: RunShortcutsInput, target: EventTarget | null): boolean {
  switch (id) {
    case 'j-next-step':
    case 'k-previous-step': {
      const next = adjacentStepSelection(
        run.steps,
        run.selectedStepId,
        id === 'j-next-step' ? 'next' : 'previous',
      );
      if (next === null) return false;
      run.selectStep(next);
      return true;
    }
    case 'enter-focus-inspector':
      return activatesOnEnter(target) ? false : focusInspectorPane(run.inspectorRef.current);
    case 'g-graph-view':
    case 't-timeline-view':
      if (!run.canShowGraph) return false;
      run.setViewMode(id === 'g-graph-view' ? 'graph' : 'timeline');
      return true;
  }
}

/**
 * A `<select>` is added on top of the shared `isEditableTarget` (audit F5)
 * rather than widened into it.
 *
 * All five of these chords are characters, and a select spends characters on
 * type-ahead: `g` over `HarnessModelPicker`'s three lists — which sit in the
 * inspector, one Tab from where `Enter` leaves the user — jumps to the `gpt-`
 * options. The shared predicate is right to leave a select alone for what its
 * other callers bind, `WorkflowBuilder`'s canvas Delete above all, so the extra
 * clause belongs to the hook whose keys need it.
 */
function typingTarget(target: EventTarget | null): boolean {
  return isEditableTarget(target) || target instanceof HTMLSelectElement;
}

/**
 * Enter is consumed with `preventDefault`, so a control that activates on it
 * has to keep it — a Retry button that stopped working while the run view was
 * open reads as a broken action, not as a shortcut. `closest` rather than the
 * element itself because a keydown on a button holding a `lucide` glyph targets
 * the `svg` inside it.
 *
 * `j`/`k`/`g`/`t` are deliberately not gated this way: they carry no native
 * meaning on a button, and gating them would strand the keys the moment Enter
 * had moved focus onto the inspector's tab strip.
 */
function activatesOnEnter(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest('button, a[href], summary') !== null;
}
