/**
 * The run view's preferences, once they outlive the mount that made them
 * (UI_REDESIGN_PLAN §6 Phase 6). `uiPrefs.test.ts` owns the store — decoding,
 * arming, the debounce. What is left, and what this file is for, is the
 * binding: that each value is restored into the surface it belongs to, and
 * that restoring one is not itself a choice.
 *
 * That second claim is the one worth a test rather than a comment. A view
 * holds a default, learns the stored value, and re-renders with it — which is
 * indistinguishable, to anything watching state, from the user picking that
 * value. Persist it from an effect on the state and every mount writes back
 * what it just read; the store keeps answering correctly and nothing looks
 * wrong until two windows disagree. So the assertion here is a *negative* one,
 * taken after the debounce window rather than immediately.
 *
 * The backend double answers only what this mount asks for and rejects the
 * rest, per `FeatureDetail.test.tsx`'s note on doubles that answer everything.
 */

import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
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
} from '../../context';
import { densityClasses } from '../../lib/density';
import {
  densityPref,
  inspectorWidthPref,
  runViewModePref,
  UI_PREF_WRITE_DEBOUNCE_MS,
} from '../../lib/uiPrefs';
import type { RunEvent, StepExecution } from '../../types';
import type { WorkflowDefinitionV2 } from '../canvas/types';
import { FeatureDetail } from './FeatureDetail';

vi.mock('react-markdown', () => ({
  default: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
}));

const FEATURE_ID = 'f-1';

const STEP: StepExecution = {
  id: 'se-1',
  feature_id: FEATURE_ID,
  step_id: 's-research',
  step_index: 0,
  step_kind: 'agent',
  status: 'completed',
  artifact_paths: [],
  created_at: 0,
  updated_at: 0,
};

const GRAPH: WorkflowDefinitionV2 = {
  schema_version: 2,
  id: 'wf-1',
  name: 'Default',
  nodes: [{ id: 's-research', type: 'agent', title: 'Research' }],
  edges: [],
};

const RUN_EVENT: RunEvent = {
  offset: 1,
  run_id: FEATURE_ID,
  kind: 'step_started',
  payload_json: null,
  created_at: 0,
};

/** The `app_session` rows this mount finds already written. */
let stored: Record<string, string> = {};
/** The workflow definition the feature's run was pinned to, or none. */
let graphDef: WorkflowDefinitionV2 | null = null;
/** Pushes one row into the local run feed, which is what makes the activity
 *  log appear at all — `RunMetaColumn` withholds an empty one. */
let emitRunEvent: (event: RunEvent) => void = () => {};

function mockBackend() {
  vi.mocked(invoke).mockImplementation(((cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case 'step_list_for_run':
        return Promise.resolve([STEP]);
      case 'sync_session_get':
        return Promise.resolve(null);
      case 'feature_get':
        return Promise.resolve({ id: FEATURE_ID, status: 'running' });
      case 'feature_workflow_graph':
        return Promise.resolve(graphDef);
      case 'get_app_session':
        return Promise.resolve(stored[String(args?.key)] ?? null);
      case 'set_app_session':
        return Promise.resolve(undefined);
      case 'feature_list_attachments':
      case 'get_machines':
      case 'list_agents':
      case 'list_terminal_sessions':
      case 'step_attempts_list':
        return Promise.resolve([]);
      case 'remote_run_for_feature':
        return Promise.resolve(null);
      default:
        return Promise.reject(new Error(`unexpected IPC command: ${cmd}`));
    }
  }) as unknown as typeof invoke);

  vi.mocked(listen).mockImplementation(((
    event: string,
    handler: (message: { payload: unknown }) => void,
  ) => {
    if (event === 'run_event') emitRunEvent = (e) => handler({ payload: e });
    return Promise.resolve(() => {});
  }) as unknown as typeof listen);
}

function Seed() {
  const { navigate } = useNavigation();
  useEffect(() => {
    navigate({ kind: 'detail', featureId: FEATURE_ID, featureTitle: 'Run' });
  }, [navigate]);
  return <FeatureDetail />;
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

function isSessionWrite(args: unknown): args is { key: string; value: string } {
  return typeof args === 'object' && args !== null && 'key' in args && 'value' in args;
}

/** Every preference write this mount has issued, in order. */
function sessionWrites(): { key: string; value: string }[] {
  return vi
    .mocked(invoke)
    .mock.calls.flatMap(([cmd, args]) =>
      cmd === 'set_app_session' && isSessionWrite(args) ? [args] : [],
    );
}

/** Past the trailing edge of the write debounce, so "nothing was written"
 *  means the write never came rather than that the assertion looked early. */
async function settleWrites(): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, UI_PREF_WRITE_DEBOUNCE_MS + 50));
  });
}

beforeEach(() => {
  stored = {};
  graphDef = null;
  emitRunEvent = () => {};
  mockBackend();
});

describe('the run view restores what was stored for it', () => {
  it('opens the timeline at the stored density', async () => {
    stored[densityPref.key] = 'compact';
    mount();

    await waitFor(() =>
      expect(screen.getByRole('list', { name: 'Run steps' })).toHaveClass(
        densityClasses('compact').list,
      ),
    );
    expect(screen.getByRole('radio', { name: /compact/i })).toHaveAttribute('aria-checked', 'true');
  });

  it('opens the activity log whatever the last mount left it on', async () => {
    mount();
    await screen.findByRole('list', { name: 'Run steps' });
    await act(async () => emitRunEvent(RUN_EVENT));

    await userEvent.click(await screen.findByRole('button', { name: /Activity/ }));
    expect(screen.getByRole('button', { name: /Activity/ })).toHaveAttribute(
      'aria-expanded',
      'false',
    );
    await settleWrites();
    // Collapsing stops `ActivityPanel`'s remote tail, and that tail is a
    // detached run's only source of bootstrap phases — so this one deliberately
    // does not survive the mount, and stores nothing that could outlive it.
    expect(sessionWrites()).toEqual([]);

    cleanup();
    mount();
    await screen.findByRole('list', { name: 'Run steps' });
    await act(async () => emitRunEvent(RUN_EVENT));

    expect(await screen.findByRole('button', { name: /Activity/ })).toHaveAttribute(
      'aria-expanded',
      'true',
    );
  });

  it('opens on the timeline when that is the stored choice, toggle and all', async () => {
    graphDef = GRAPH;
    stored[runViewModePref.key] = 'timeline';
    mount();

    // The toggle being *offered* is what separates a stored choice from the
    // no-definition fallback below: this run could have shown a graph.
    expect(await screen.findByRole('radiogroup', { name: 'Run view' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /timeline/i })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    expect(screen.getByRole('list', { name: 'Run steps' })).toBeInTheDocument();
  });

  it('does not store the timeline it falls back to without a definition', async () => {
    stored[runViewModePref.key] = 'graph';
    mount();

    // `canShowGraph` sits downstream of the stored value (UI_REDESIGN_PLAN §7),
    // so a legacy run opens on the timeline with nothing to switch to. The trap
    // is in *how*: spelling that fallback as `setViewMode('timeline')` reads
    // identically on screen, and the preference is global — so opening one
    // graphless feature would move every future run off the graph.
    expect(await screen.findByRole('list', { name: 'Run steps' })).toBeInTheDocument();
    expect(screen.queryByRole('radiogroup', { name: 'Run view' })).not.toBeInTheDocument();
    await settleWrites();
    expect(sessionWrites()).toEqual([]);
  });
});

describe('what the run view stores back', () => {
  it('writes nothing for merely having restored its preferences', async () => {
    graphDef = GRAPH;
    stored[densityPref.key] = 'compact';
    stored[runViewModePref.key] = 'timeline';
    stored[inspectorWidthPref.key] = '640';
    mount();

    await screen.findByRole('list', { name: 'Run steps' });
    await settleWrites();

    expect(sessionWrites()).toEqual([]);
  });

  it('writes the choice the user made, under that preference’s own key', async () => {
    mount();

    await userEvent.click(await screen.findByRole('radio', { name: /compact/i }));
    await waitFor(() =>
      expect(sessionWrites()).toEqual([{ key: densityPref.key, value: 'compact' }]),
    );

    // One choice is one write: a second flush would mean something other than
    // the user's own setter reaches the store.
    await settleWrites();
    expect(sessionWrites()).toEqual([{ key: densityPref.key, value: 'compact' }]);
  });
});
