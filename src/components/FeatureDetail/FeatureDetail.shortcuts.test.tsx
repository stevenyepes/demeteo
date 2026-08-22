/**
 * The run view's keys, wired (UI_REDESIGN_PLAN §3.6).
 *
 * `useRunShortcuts.test.tsx` owns what each id does against spies; this file
 * owns the two joins that no spy can stand in for. A key moves the selection
 * that *the run already has* — held on the `detail` view, not in a second copy
 * beside it (§3.5) — so the claim is that the row on screen changes, and
 * `Enter` aims at a real element in the real inspector, so the claim is that
 * focus lands inside the pane the component actually rendered.
 *
 * The backend double answers only what this mount asks for and rejects the
 * rest, per `FeatureDetail.test.tsx`'s note on doubles that answer everything.
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useEffect, type ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import {
  NavigationProvider,
  ProjectProvider,
  TerminalPanelProvider,
  UIStateProvider,
  useNavigation,
  useUIState,
} from '../../context';
import type { StepExecution } from '../../types';
import { FeatureDetail } from './FeatureDetail';

vi.mock('react-markdown', () => ({
  default: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
}));

const FEATURE_ID = 'f-1';

function step(id: string, stepId: string, index: number): StepExecution {
  return {
    id,
    feature_id: FEATURE_ID,
    step_id: stepId,
    step_index: index,
    step_kind: 'agent',
    status: 'completed',
    artifact_paths: [],
    created_at: 0,
    updated_at: 0,
  };
}

const STEPS = [
  step('se-1', 'research', 0),
  step('se-2', 'implement', 1),
  step('se-3', 'review', 2),
];

function mockBackend() {
  vi.mocked(invoke).mockImplementation(((cmd: string) => {
    switch (cmd) {
      case 'step_list_for_run':
        return Promise.resolve(STEPS);
      case 'sync_session_get':
        return Promise.resolve(null);
      case 'feature_get':
        return Promise.resolve({ id: FEATURE_ID, status: 'completed' });
      case 'feature_workflow_graph':
      case 'get_app_session':
      case 'remote_run_for_feature':
        return Promise.resolve(null);
      case 'set_app_session':
        return Promise.resolve(undefined);
      case 'feature_list_attachments':
      case 'get_machines':
      case 'list_agents':
      case 'list_terminal_sessions':
      case 'step_attempts_list':
      case 'step_artifacts_list':
        return Promise.resolve([]);
      default:
        return Promise.reject(new Error(`unexpected IPC command: ${cmd}`));
    }
  }) as unknown as typeof invoke);

  vi.mocked(listen).mockImplementation((() =>
    Promise.resolve(() => {})) as unknown as typeof listen);
}

function Seed() {
  const { navigate } = useNavigation();
  const { uiDispatch } = useUIState();
  useEffect(() => {
    navigate({ kind: 'detail', featureId: FEATURE_ID, featureTitle: 'Run' });
  }, [navigate]);
  return (
    <>
      {/* `App.tsx` mounts the palette, the docs panel and the start-feature
          modal as siblings of this component rather than inside it, so opening
          one leaves the run view mounted and its window listener live. That is
          the arrangement under test; this stands in for the trigger. */}
      <button type="button" onClick={() => uiDispatch({ type: 'SET_DOCS_PANEL', open: true })}>
        open docs
      </button>
      <FeatureDetail />
    </>
  );
}

function mount() {
  return render(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <TerminalPanelProvider>
            <Seed />
          </TerminalPanelProvider>
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );
}

/** The step the timeline is marking as selected — `aria-current="step"`, which
 *  is what `StepCard` carries and therefore what a user can see. */
function currentStepId(): string | null {
  return document.querySelector('[aria-current="step"]')?.getAttribute('data-step-row') ?? null;
}

beforeEach(() => {
  mockBackend();
});

describe('the run view drives its own selection from the keyboard', () => {
  it('moves the selection the run already has, rather than a copy beside it', async () => {
    mount();
    // A finished run opens on its last settled step (`defaultInspectorSelection`).
    await waitFor(() => expect(currentStepId()).toBe('se-3'));

    fireEvent.keyDown(document.body, { key: 'k' });
    await waitFor(() => expect(currentStepId()).toBe('se-2'));
    // The one inspector reads the same selection (§3.1), so a key that moved a
    // parallel copy of it would leave this heading behind.
    expect(within(screen.getByTestId('inspector')).getByRole('heading', { level: 3 }))
      .toHaveTextContent(/implement/i);

    fireEvent.keyDown(document.body, { key: 'j' });
    await waitFor(() => expect(currentStepId()).toBe('se-3'));
  });

  /** Enter aims at the column's roving entry point, and the column gained an
   *  outer strip: Step/Sync now sits above the card, so the first
   *  `[role="tab"][tabindex="0"]` inside the wrapper is the pane switch rather
   *  than the step inspector's Overview. That is the outermost choice in the
   *  column and the right first stop — and it is a behaviour change, which is
   *  why it is asserted here rather than left to be noticed. */
  it('lands Enter on the pane switch the column opens with', async () => {
    mount();
    const column = await screen.findByTestId('inspector-column');

    fireEvent.keyDown(document.body, { key: 'Enter' });
    expect(column.contains(document.activeElement)).toBe(true);
    expect(document.activeElement).toHaveAccessibleName('Step');
  });

  it('goes quiet under an overlay this view does not mount', async () => {
    mount();
    await waitFor(() => expect(currentStepId()).toBe('se-3'));

    fireEvent.click(screen.getByRole('button', { name: 'open docs' }));

    // None of these overlays moves focus, so `document.body` keeps it and the
    // editable-target guard alone lets every key through. The run is occluded,
    // so a selection that moved would be invisible — and `t` is worse than
    // invisible: `runViewModePref` is global, so a surface the user cannot see
    // would choose the opening view for every future run.
    fireEvent.keyDown(document.body, { key: 'k' });
    fireEvent.keyDown(document.body, { key: 't' });
    fireEvent.keyDown(document.body, { key: 'Enter' });

    await waitFor(() => expect(currentStepId()).toBe('se-3'));
    expect(screen.getByTestId('inspector').contains(document.activeElement)).toBe(false);
  });
});
