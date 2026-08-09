/**
 * The selection lives on the view, and getting there must cost nothing.
 *
 * Two claims, and the second is the one a reader is likely to undo. Selection
 * is navigation state (UI_REDESIGN_PLAN §3.5), so it looks like every other
 * `navigate` call in the app — and every other one pushes. Pushing here puts a
 * history entry behind each row the user glances at, so Back walks the run step
 * by step instead of leaving it. Nothing in `NavigationContext` prevents that;
 * only the `'replace'` argument does, and only a test keeps it.
 *
 * The real reducer drives all of this rather than a `navigate` spy: the stacks
 * and `shallowEqualView` are what the claims are *about*, and asserting on the
 * argument passed to a double would leave both untested.
 */
import { act, render } from '@testing-library/react';
import { useEffect } from 'react';
import { describe, expect, it } from 'vitest';

import { NavigationProvider, useNavigation } from '../../context';
import type { NavigationMode } from '../../context/NavigationContext';
import type { AppView, StepExecution } from '../../types';
import { useStepSelection, type StepSelection } from './useStepSelection';

const FEATURE_ID = 'f-1';
type DetailView = Extract<AppView, { kind: 'detail' }>;

const step = (over: Partial<StepExecution>): StepExecution => ({
  id: 'se-1',
  feature_id: FEATURE_ID,
  step_id: 's-research',
  step_index: 0,
  step_kind: 'agent',
  status: 'completed',
  artifact_paths: [],
  created_at: 0,
  updated_at: 0,
  ...over,
});

const STEPS: StepExecution[] = [
  step({ id: 'se-1', step_id: 's-research', step_index: 0 }),
  step({ id: 'se-2', step_id: 's-implement', step_index: 1, status: 'failed' }),
  step({ id: 'se-3', step_id: 's-review', step_index: 2, status: 'pending' }),
];

interface Probe {
  selection: StepSelection;
  view: DetailView;
  navigate: (view: AppView, mode?: NavigationMode) => void;
  goBack: () => void;
  goForward: () => void;
}

/** The shape every gate route builds: `featureId` and the title, and whatever
 *  the route itself is for. None of them carry a selection forward. */
const detailView = (over: Partial<DetailView> = {}): AppView => ({
  kind: 'detail',
  featureId: FEATURE_ID,
  featureTitle: 'Run',
  ...over,
});

function mount(options: { steps?: StepExecution[]; seed?: Partial<DetailView> } = {}) {
  const steps = options.steps ?? STEPS;
  const probe: { current: Probe | null } = { current: null };
  /** Tracked one level above `Inner`, which unmounts the moment the view stops
   *  being a detail view — the state a Back out of the run lands in. */
  const live: { view: AppView | null } = { view: null };

  function Harness() {
    const { view, navigate, goBack, goForward } = useNavigation();
    live.view = view;
    useEffect(() => {
      navigate(detailView(options.seed));
    }, [navigate]);

    if (view.kind !== 'detail') return null;
    return <Inner view={view} navigate={navigate} goBack={goBack} goForward={goForward} />;
  }

  function Inner({
    view,
    navigate,
    goBack,
    goForward,
  }: {
    view: DetailView;
    navigate: (view: AppView, mode?: NavigationMode) => void;
    goBack: () => void;
    goForward: () => void;
  }) {
    const selection = useStepSelection({ view, steps, navigate });
    probe.current = { selection, view, navigate, goBack, goForward };
    return null;
  }

  render(
    <NavigationProvider>
      <Harness />
    </NavigationProvider>,
  );

  const read = (): Probe => {
    if (!probe.current) throw new Error('the harness never reached a detail view');
    return probe.current;
  };
  return { read, liveView: () => live.view, run: (fn: () => void) => act(fn) };
}

describe('useStepSelection', () => {
  it('opens an untouched run on the step that needs the reader', () => {
    const { read } = mount();
    // `defaultInspectorSelection` prefers the failure over the pending step;
    // the claim here is that seeding happened at all.
    expect(read().view.selectedStepId).toBe('se-2');
    expect(read().selection.target.kind).toBe('step');
  });

  it('leaves a selection that arrived with the view alone', () => {
    // A deep link or a Back into a run the user had already been reading.
    const { read } = mount({ seed: { selectedStepId: 'se-1' } });
    expect(read().view.selectedStepId).toBe('se-1');
  });

  it('reports an empty run rather than seeding one', () => {
    const { read } = mount({ steps: [] });
    expect(read().selection.target).toEqual({ kind: 'empty', reason: 'no-steps' });
    expect(read().view.selectedStepId).toBeUndefined();
  });

  it('does not push a history entry per selection', () => {
    const { read, run, liveView } = mount();
    run(() => read().selection.selectStep('se-1'));
    run(() => read().selection.selectStep('se-3'));
    run(() => read().selection.selectStep('se-2'));
    expect(read().view.selectedStepId).toBe('se-2');

    // `canGoBack` cannot carry this claim — it was already true from the one
    // navigation that opened the run, and stays true however many entries pile
    // up behind it. Walking the stack is what distinguishes one entry from
    // four: one Back has to leave the run entirely.
    run(() => read().goBack());
    expect(liveView()?.kind).toBe('empty-state');

    // And Forward returns to the run as the user left it, rather than to the
    // first of a stack of rows they only glanced at.
    run(() => read().goForward());
    expect(read().view.selectedStepId).toBe('se-2');
  });

  it('keeps a pending gate through a selection change', () => {
    // The gate overlay reads the same view object. A writer that rebuilt it
    // from `featureId` alone would dismiss the gate on a row click.
    const { read, run } = mount({ seed: { gateStepExecutionId: 'se-2' } });
    run(() => read().selection.selectStep('se-3'));
    expect(read().view.gateStepExecutionId).toBe('se-2');
    expect(read().view.selectedStepId).toBe('se-3');
  });

  it('resolves one selection into the id each surface needs', () => {
    const { read, run } = mount();
    run(() => read().selection.selectStep('s-review'));
    const { target, selectedNodeId, selectedExecutionId } = read().selection;
    expect(target.kind === 'step' && target.step.id).toBe('se-3');
    expect(selectedNodeId).toBe('s-review');
    expect(selectedExecutionId).toBe('se-3');
  });

  it('clears the selection when the step already shown is activated again', () => {
    const { read, run } = mount();
    run(() => read().selection.toggleStep('se-1'));
    expect(read().view.selectedStepId).toBe('se-1');
    // By node id as well as by execution id: the canvas activates with one and
    // the timeline with the other, and both mean "this is already open".
    run(() => read().selection.toggleStep('s-research'));
    expect(read().view.selectedStepId).toBeNull();
  });

  it('does not re-seed after a deselect', () => {
    const { read, run } = mount();
    run(() => read().selection.selectStep(null));
    expect(read().view.selectedStepId).toBeNull();
    expect(read().selection.target).toEqual({ kind: 'empty', reason: 'no-selection' });
  });

  it('comes back from a gate with a step in the inspector', () => {
    // Every gate route rebuilds the detail view from `featureId` alone, and
    // `FeatureDetail` stays mounted underneath the overlay throughout — so the
    // hook sees a view with no selection twice, on the way in and on the way
    // out, and neither is the user asking for an empty pane.
    const { read, run } = mount();
    run(() => read().selection.selectStep('se-1'));

    run(() => read().navigate(detailView({ gateStepExecutionId: 'se-2' })));
    expect(read().selection.target.kind).toBe('step');
    // Seeding navigates, and a writer that rebuilt the view instead of
    // extending it would close the gate the user is standing in.
    expect(read().view.gateStepExecutionId).toBe('se-2');

    run(() => read().navigate(detailView()));
    expect(read().view.selectedStepId).toBe('se-2');
    expect(read().selection.target.kind).toBe('step');
  });

  it('tells a cleared selection apart from one no navigation set', () => {
    const { read, run } = mount();
    run(() => read().selection.selectStep(null));
    expect(read().selection.target).toEqual({ kind: 'empty', reason: 'no-selection' });

    // Same absent selection as far as the inspector is concerned; different
    // provenance, and only the view distinguishes them.
    run(() => read().navigate(detailView()));
    expect(read().selection.target.kind).toBe('step');
  });

  it('names a selection the run no longer has', () => {
    const { read, run } = mount();
    run(() => read().selection.selectStep('se-gone'));
    expect(read().selection.target).toEqual({ kind: 'empty', reason: 'stale-selection' });
  });

  it('keeps its writers stable so a click re-renders no memoized row', () => {
    const { read, run } = mount();
    const before = read().selection;
    run(() => read().selection.selectStep('se-3'));
    const after = read().selection;
    expect(after.selectStep).toBe(before.selectStep);
    expect(after.toggleStep).toBe(before.toggleStep);
  });
});
