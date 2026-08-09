/**
 * The graph's Live tab gets the selected step's stream, not a stranded prop.
 *
 * Both run surfaces read the same per-step subscription, and they reach it by
 * different routes: the timeline subscribes for the card it has open, while the
 * graph path subscribes here in `FeatureDetail` and hands the text down through
 * `RunGraphPanel` to `NodePanel`. Two consumers of one store is the shape where
 * fixing one and stranding the other stays green — nothing else asserts that
 * the graph route carries text at all, and the key it subscribes with (a step
 * *execution* id, matching `agent_stream`'s `step_execution_id`) is invisible
 * from either end.
 *
 * `RunGraphPanel` stands in for the canvas rather than rendering it: the real
 * one mounts xyflow and runs ELK, none of which this claim depends on. The stub
 * exposes the two things it does depend on — a way to activate a node, and what
 * `liveStream` arrived as.
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
  RunGraphPanel: ({
    liveStream,
    onNodeActivate,
  }: {
    liveStream: string | undefined;
    onNodeActivate: (nodeId: string) => void;
  }) => (
    <div>
      <button type="button" onClick={() => onNodeActivate(NODE_ID)}>
        activate node
      </button>
      <div data-testid="live-stream">{liveStream}</div>
    </div>
  ),
}));

let timelineRenders = 0;
vi.mock('./StepTimeline', () => ({
  StepTimeline: () => {
    timelineRenders += 1;
    return null;
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
        return Promise.resolve([]);
      case 'remote_run_for_feature':
        return Promise.resolve(null);
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

beforeEach(() => {
  for (const key of Object.keys(handlers)) delete handlers[key];
  timelineRenders = 0;
  mockBackend();
});

describe('the graph path’s live stream', () => {
  it('reaches RunGraphPanel for the selected step', async () => {
    const user = userEvent.setup();
    mount();

    await user.click(screen.getByText('open detail'));
    await waitFor(() => expect(screen.getByText('Graph')).toBeInTheDocument());
    await user.click(screen.getByText('Graph'));
    await user.click(await screen.findByText('activate node'));

    expect(screen.getByTestId('live-stream')).toHaveTextContent('');

    await emitChunk('reading src/lib/runStatus.ts');

    // Fails if the subscription keys off the node id, a stale selection, or
    // nothing at all — each of which leaves the timeline path working.
    await waitFor(() =>
      expect(screen.getByTestId('live-stream')).toHaveTextContent('reading src/lib/runStatus.ts'),
    );
  });

  it('does not wake the detail view while the timeline is the active surface', async () => {
    const user = userEvent.setup();
    mount();

    await user.click(screen.getByText('open detail'));
    await waitFor(() => expect(screen.getByText('Graph')).toBeInTheDocument());
    // Select a node, then leave the graph. The selection outlives the view
    // switch, so this is the only state where the mode gate is what decides
    // whether a subscription exists — with nothing selected there is no
    // subscription either way and the assertion below would prove nothing.
    await user.click(screen.getByText('Graph'));
    await user.click(await screen.findByText('activate node'));
    await user.click(screen.getByText('Timeline'));
    const settled = timelineRenders;

    await emitChunk('output nobody is watching');

    // Asserting `RunGraphPanel` is absent would prove nothing — it is unmounted
    // in timeline mode whether or not the subscription exists. The subscription
    // is only observable through what it wakes: an ungated one re-renders
    // `FeatureDetailView`, and with it the timeline, once per frame for a
    // surface nobody is looking at.
    expect(timelineRenders).toBe(settled);
  });
});
