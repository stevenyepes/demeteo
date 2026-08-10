/**
 * The five run-view keys, and the one thing that makes them dangerous.
 *
 * `j`, `k`, `g` and `t` are characters and `Enter` submits things, so every one
 * of them is bare `?` waiting to happen — the chord that used to be consumed
 * with `preventDefault` out of whatever field had focus (audit F5). That is why
 * the guard cases here outnumber the behaviours, and why they are asserted key
 * by key rather than for one representative: a dispatcher that grew a sixth
 * chord past the guard would still pass a single-key version of this file.
 *
 * Nothing below names a key the registry does not. The chord for each id is
 * `shortcuts.test.ts`'s business; what these tests own is the mapping from a
 * registry entry to an action, and the states in which that mapping is refused.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { useRef } from 'react';
import { describe, expect, it, vi } from 'vitest';

import type { StepExecution } from '../../types';
import { RUN_SHORTCUT_ENTRIES, useRunShortcuts } from './useRunShortcuts';

function step(over: Partial<StepExecution> & Pick<StepExecution, 'id'>): StepExecution {
  return {
    feature_id: 'f-1',
    step_id: over.id,
    step_index: 0,
    step_kind: 'agent',
    status: 'completed',
    artifact_paths: [],
    created_at: 1,
    updated_at: 1,
    ...over,
  };
}

const STEPS: StepExecution[] = [
  step({ id: 'e-1', step_index: 0 }),
  step({ id: 'e-2', step_index: 1 }),
  step({ id: 'e-3', step_index: 2 }),
];

/** Every chord in this file, spelled once, so a key appears in a test only as
 *  the thing being pressed. */
const KEY = { next: 'j', previous: 'k', focus: 'Enter', graph: 'g', timeline: 't' } as const;

interface Options {
  enabled?: boolean;
  steps?: StepExecution[];
  selectedStepId?: string | null;
  canShowGraph?: boolean;
  /** `false` mounts the pane with no tab strip — `InspectorEmpty`'s shape. */
  tabs?: boolean;
}

function mount(options: Options = {}) {
  const selectStep = vi.fn();
  const setViewMode = vi.fn();

  function Harness() {
    const inspectorRef = useRef<HTMLDivElement | null>(null);
    useRunShortcuts({
      enabled: options.enabled ?? true,
      steps: options.steps ?? STEPS,
      selectedStepId: options.selectedStepId === undefined ? 'e-2' : options.selectedStepId,
      selectStep,
      inspectorRef,
      canShowGraph: options.canShowGraph ?? true,
      setViewMode,
    });

    return (
      <>
        <input aria-label="query" />
        <textarea aria-label="prompt" />
        <div
          aria-label="notes"
          contentEditable
          ref={(el) => {
            // jsdom does not derive `isContentEditable` from the attribute.
            if (el) Object.defineProperty(el, 'isContentEditable', { value: true });
          }}
        />
        <select aria-label="model">
          <option value="a">a</option>
        </select>
        <button type="button">Retry</button>
        <div ref={inspectorRef} tabIndex={-1} data-testid="pane">
          {(options.tabs ?? true) && (
            <div role="tablist">
              <button type="button" role="tab" tabIndex={-1}>
                Overview
              </button>
              <button type="button" role="tab" tabIndex={0}>
                Live
              </button>
            </div>
          )}
        </div>
      </>
    );
  }

  const { unmount } = render(<Harness />);
  return { selectStep, setViewMode, unmount };
}

/** `dispatchEvent` answers `false` for a consumed event, so this is the same
 *  question as "did the dispatcher call `preventDefault`" — which is the half
 *  of a swallowed keystroke that the absent side effect cannot show. */
function press(key: string, target: Element | Document = document.body): boolean {
  return fireEvent.keyDown(target, { key, bubbles: true });
}

describe('the run view keys', () => {
  it('moves the selection down and up the run', () => {
    const { selectStep } = mount();

    expect(press(KEY.next)).toBe(false);
    expect(selectStep).toHaveBeenLastCalledWith('e-3');

    expect(press(KEY.previous)).toBe(false);
    expect(selectStep).toHaveBeenLastCalledWith('e-1');
  });

  it('leaves the key alone at the end of the run rather than wrapping', () => {
    const { selectStep } = mount({ selectedStepId: 'e-3' });

    // Untouched, not merely inert: nothing else can bind `j` while a dispatcher
    // consumes it for a move it did not make.
    expect(press(KEY.next)).toBe(true);
    expect(selectStep).not.toHaveBeenCalled();
  });

  it('switches the run surface', () => {
    const { setViewMode } = mount();

    expect(press(KEY.graph)).toBe(false);
    expect(setViewMode).toHaveBeenLastCalledWith('graph');

    expect(press(KEY.timeline)).toBe(false);
    expect(setViewMode).toHaveBeenLastCalledWith('timeline');
  });

  it('does not switch a run that has no graph to switch to', () => {
    // `RunViewToggle` is not rendered either, so a fired `g` would change a
    // *global* preference from a surface showing no sign of having changed.
    const { setViewMode } = mount({ canShowGraph: false });

    expect(press(KEY.graph)).toBe(true);
    expect(press(KEY.timeline)).toBe(true);
    expect(setViewMode).not.toHaveBeenCalled();
  });

  it('focuses the inspector at its tab strip’s roving entry', () => {
    mount();

    expect(press(KEY.focus)).toBe(false);
    expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'Live' }));
  });

  it('focuses the pane itself when the inspector has no tabs', () => {
    mount({ tabs: false });

    expect(press(KEY.focus)).toBe(false);
    expect(document.activeElement).toBe(screen.getByTestId('pane'));
  });

  it('does nothing on a run whose steps have not arrived', () => {
    const { selectStep } = mount({ steps: [], selectedStepId: null });

    expect(press(KEY.next)).toBe(true);
    expect(press(KEY.previous)).toBe(true);
    expect(selectStep).not.toHaveBeenCalled();
  });

  it('stops firing once the view is gone', () => {
    // The listener is on `window`, so an unmount that left it bound would keep
    // driving the navigation state of a run nobody is looking at.
    const { selectStep, unmount } = mount();
    unmount();

    press(KEY.next);
    expect(selectStep).not.toHaveBeenCalled();
  });
});

describe('the keys stay out of anything the user is typing into', () => {
  const fields = [
    ['an input', () => screen.getByLabelText('query')],
    ['a textarea', () => screen.getByLabelText('prompt')],
    ['a contenteditable', () => screen.getByLabelText('notes')],
  ] as const;

  it.each(fields)('leaves every run key alone in %s', (_name, field) => {
    const { selectStep, setViewMode } = mount();
    const before = document.activeElement;

    for (const key of Object.values(KEY)) {
      expect(press(key, field())).toBe(true);
    }

    expect(selectStep).not.toHaveBeenCalled();
    expect(setViewMode).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(before);
  });

  it('leaves a select its type-ahead', () => {
    // `HarnessModelPicker` seats three of these in the inspector, one Tab from
    // where `Enter` leaves the user; `g` there means "jump to the gpt- rows".
    // `isEditableTarget` deliberately does not claim a select, so this guard is
    // the hook's own and disappears the moment someone inlines the shared one.
    const { selectStep, setViewMode } = mount();
    const model = screen.getByLabelText('model');

    for (const key of Object.values(KEY)) {
      expect(press(key, model)).toBe(true);
    }

    expect(selectStep).not.toHaveBeenCalled();
    expect(setViewMode).not.toHaveBeenCalled();
  });

  it('leaves a button the Enter that activates it', () => {
    // Consumed here, the inspector's own Retry would stop responding to the
    // keyboard — a broken action, not a missing shortcut.
    mount();
    const retry = screen.getByRole('button', { name: 'Retry' });

    expect(press(KEY.focus, retry)).toBe(true);
    expect(document.activeElement).not.toBe(screen.getByRole('tab', { name: 'Live' }));

    // The other four carry no meaning on a button, and gating them there would
    // strand the user the moment Enter had moved focus onto the tab strip.
    expect(press(KEY.next, retry)).toBe(false);
  });
});

describe('the keys stand down for whatever is over the run', () => {
  it('fires nothing while an overlay is open', () => {
    const { selectStep, setViewMode } = mount({ enabled: false });

    for (const key of Object.values(KEY)) {
      expect(press(key)).toBe(true);
    }

    expect(selectStep).not.toHaveBeenCalled();
    expect(setViewMode).not.toHaveBeenCalled();
    expect(document.activeElement).not.toBe(screen.getByRole('tab', { name: 'Live' }));
  });
});

describe('the registry is the only place a chord is spelled', () => {
  it('resolves every id this hook binds', () => {
    // A rename in the registry drops an id out of the lookup silently: the key
    // fires nothing while the help overlay carries on advertising it.
    expect(RUN_SHORTCUT_ENTRIES.map(([id]) => id)).toEqual([
      'j-next-step',
      'k-previous-step',
      'enter-focus-inspector',
      'g-graph-view',
      't-timeline-view',
    ]);
  });

  it('stands down for a run surface that claimed the key first', () => {
    // The graph canvas binds Enter to activate the selected node and calls
    // `preventDefault`; React delegates at the root, so it has already run by
    // the time this window listener sees the event. Answering anyway made one
    // Enter toggle the node's selection off and then focus the inspector it had
    // just emptied — the opposite of the advertised action.
    const { selectStep, setViewMode } = mount();

    for (const key of Object.values(KEY)) {
      const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });
      event.preventDefault();
      document.body.dispatchEvent(event);
    }

    expect(selectStep).not.toHaveBeenCalled();
    expect(setViewMode).not.toHaveBeenCalled();
    expect(document.activeElement).not.toBe(screen.getByTestId('pane'));
  });

  it('does not answer the Cmd/Ctrl chords that share its letters', () => {
    // `Cmd/Ctrl + G` is next-feature and `Cmd/Ctrl + T` is New Feature. They
    // stay separate because `matchesKeyEvent` compares `primary` exactly.
    const { selectStep, setViewMode } = mount();

    fireEvent.keyDown(document.body, { key: 'g', metaKey: true });
    fireEvent.keyDown(document.body, { key: 't', ctrlKey: true });
    fireEvent.keyDown(document.body, { key: 'j', altKey: true });
    fireEvent.keyDown(document.body, { key: 'k', shiftKey: true });

    expect(selectStep).not.toHaveBeenCalled();
    expect(setViewMode).not.toHaveBeenCalled();
  });
});
