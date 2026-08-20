/**
 * One inspector, both run surfaces, and a stream that wakes only it.
 *
 * Two claims, and the mount has to be this wide for either. One inspector
 * serves both surfaces, so neither surface may leave its Live tab empty; and
 * the run around it must stay asleep while chunks arrive, which is the ceiling
 * `StepInspector.tsx`'s header argues for. That argument is why the render
 * count is taken from the whole detail view here —
 * `StepTimeline.rerender.test.tsx` mounts the timeline alone, so a subscription
 * placed above it is exactly what that file cannot see.
 *
 * `RunGraphPanel` and `StepTimeline` stand in for the two surfaces rather than
 * rendering them: the real canvas mounts xyflow and runs ELK, and the real
 * timeline is a second render-count subject. Each stub exposes only what the
 * claims depend on — a way to select a step, and (for the timeline) a render
 * counter. The inspector itself is real, because it is the thing under test.
 */
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

const handlers: Record<string, Array<(e: { payload: unknown }) => void>> = {};
vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => {
    (handlers[event] ??= []).push(cb);
    return Promise.resolve(() => {
      handlers[event] = (handlers[event] ?? []).filter((h) => h !== cb);
    });
  },
}));

vi.mock('react-markdown', () => ({
  default: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
}));

vi.mock('./RunGraphPanel', () => ({
  RunGraphPanel: ({ onNodeActivate }: { onNodeActivate: (nodeId: string) => void }) => (
    <button type="button" onClick={() => onNodeActivate(NODE_ID)}>
      activate node
    </button>
  ),
}));

let timelineRenders = 0;
vi.mock('./StepTimeline', () => ({
  StepTimeline: ({ onSelect }: { onSelect: (stepExecutionId: string) => void }) => {
    timelineRenders += 1;
    return (
      <button type="button" onClick={() => onSelect(STEP_EXECUTION_ID)}>
        select row
      </button>
    );
  },
}));

import {
  NavigationProvider,
  ProjectProvider,
  TerminalPanelProvider,
  UIStateProvider,
  useNavigation,
} from '../../context';
import type { StepExecution } from '../../types';
import { FeatureDetail } from './FeatureDetail';

const FEATURE_ID = 'f-1';
const NODE_ID = 's-implement';
const STEP_EXECUTION_ID = 'se-9';

const RUNNING_STEP: StepExecution = {
  id: STEP_EXECUTION_ID,
  feature_id: FEATURE_ID,
  step_id: NODE_ID,
  step_index: 0,
  step_kind: 'agent',
  status: 'running',
  artifact_paths: [],
  created_at: 0,
  updated_at: 0,
};

/** Answers only what this mount asks for; anything else rejects rather than
 *  resolving to a bland `undefined` the component renders around. */
function mockBackend() {
  vi.mocked(invoke).mockImplementation(((cmd: string) => {
    switch (cmd) {
      case 'step_list_for_run':
        return Promise.resolve([RUNNING_STEP]);
      case 'sync_session_get':
        return Promise.resolve(null);
      case 'feature_get':
        return Promise.resolve({ id: FEATURE_ID, status: 'running' });
      case 'feature_workflow_graph':
        return Promise.resolve({
          schema_version: 2,
          nodes: [{ id: NODE_ID, kind: 'agent', title: 'Implement', position: { x: 0, y: 0 } }],
          edges: [],
        });
      case 'feature_list_attachments':
      case 'get_machines':
      case 'list_agents':
      case 'list_terminal_sessions':
      case 'run_events_since':
      case 'step_attempts_list':
        return Promise.resolve([]);
      case 'remote_run_for_feature':
      case 'get_app_session':
        return Promise.resolve(null);
      case 'set_app_session':
        return Promise.resolve(undefined);
      default:
        return Promise.reject(new Error(`unexpected IPC command: ${cmd}`));
    }
  }) as unknown as typeof invoke);
}

function Seed() {
  const { navigate } = useNavigation();
  return (
    <>
      <button
        type="button"
        onClick={() => navigate({ kind: 'detail', featureId: FEATURE_ID, featureTitle: 'Run' })}
      >
        open detail
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

function emitChunk(content: string) {
  return act(async () => {
    for (const handler of handlers.agent_stream ?? []) {
      handler({
        payload: { feature_id: FEATURE_ID, step_execution_id: STEP_EXECUTION_ID, content },
      });
    }
    // The store coalesces its wake to one animation frame.
    await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
  });
}

/** The inspector opens on Overview; the stream lives one tab over. */
async function openLiveTab(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole('tab', { name: 'Live' }));
}

beforeEach(() => {
  for (const key of Object.keys(handlers)) delete handlers[key];
  timelineRenders = 0;
  mockBackend();
});

describe('the inspector’s live stream', () => {
  it('carries the selected step’s output in graph mode', async () => {
    const user = userEvent.setup();
    mount();

    await user.click(screen.getByText('open detail'));
    // The run opens with its live step already selected, so the first
    // activation clears it and the second re-selects — by *node* id, which is
    // the other of the two id flavours one selection key has to serve.
    await user.click(await screen.findByText('activate node'));
    expect(await screen.findByTestId('inspector-empty')).toBeInTheDocument();
    await user.click(screen.getByText('activate node'));
    await openLiveTab(user);

    await emitChunk('reading src/lib/runStatus.ts');

    // Fails if the subscription keys off the node id, a stale selection, or
    // nothing at all — none of which the surface itself would reveal.
    await waitFor(() =>
      expect(screen.getByTestId('inspector')).toHaveTextContent('reading src/lib/runStatus.ts'),
    );
  });

  it('carries it in timeline mode too', async () => {
    const user = userEvent.setup();
    mount();

    await user.click(screen.getByText('open detail'));
    await waitFor(() => expect(screen.getByText('Timeline')).toBeInTheDocument());
    await user.click(screen.getByText('Timeline'));
    await user.click(await screen.findByText('select row'));
    await openLiveTab(user);

    await emitChunk('running cargo test');

    // The timeline reaches the same buffer the graph does: which surface is
    // showing is not something the subscription is allowed to know.
    await waitFor(() =>
      expect(screen.getByTestId('inspector')).toHaveTextContent('running cargo test'),
    );
  });

  it('does not wake the detail view around it', async () => {
    const user = userEvent.setup();
    mount();

    await user.click(screen.getByText('open detail'));
    await waitFor(() => expect(screen.getByText('Timeline')).toBeInTheDocument());
    await user.click(screen.getByText('Timeline'));
    await user.click(await screen.findByText('select row'));
    await openLiveTab(user);
    // Prove the subscription is live before asserting on what it does *not*
    // do — otherwise a component that subscribed to nothing would pass.
    await emitChunk('first');
    await waitFor(() => expect(screen.getByTestId('inspector')).toHaveTextContent('first'));

    const settled = timelineRenders;
    for (let i = 0; i < 5; i += 1) await emitChunk(`chunk ${i}\n`);
    await waitFor(() => expect(screen.getByTestId('inspector')).toHaveTextContent('chunk 4'));

    expect(timelineRenders).toBe(settled);
  });
});
