/**
 * The pane that is always there, including when it has nothing to show.
 *
 * The inspector never collapses (UI_REDESIGN_PLAN §7), so its empty state is on
 * screen for a real share of a run's life. That is why `inspectorTarget` splits
 * "empty" three ways and why these tests read the wording rather than a test id:
 * one shared "nothing selected" for all three is precisely how a permanent
 * empty pane comes to read as a broken one.
 */
import { act, render, screen, cleanup, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../ArtifactViewer', () => ({
  ArtifactViewer: () => null,
}));

/** The Overview tab's most expensive child, counted rather than rendered: it
 *  JSON-parses and time-formats every row of a 500-event feed, so it is what a
 *  stream-driven re-render of a tab that reads no stream actually costs. */
const feed = { renders: 0 };
vi.mock('../RunEventFeed', () => ({
  RunEventFeed: () => {
    feed.renders += 1;
    return null;
  },
}));

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { StepInspector } from './StepInspector';
import type { AgentStreamStore } from './useAgentStream';
import type { InspectorTarget } from '../../lib/inspectorTarget';
import type { WorkflowDefinitionV2 } from '../canvas/types';
import type { RunEvent, StepExecution } from '../../types';

const step = (over: Partial<StepExecution> = {}): StepExecution => ({
  id: 'se-1',
  feature_id: 'f-1',
  step_id: 's-implement',
  step_index: 1,
  step_kind: 'agent',
  status: 'failed',
  artifact_paths: [],
  created_at: 0,
  updated_at: 0,
  ...over,
});

const GRAPH: WorkflowDefinitionV2 = {
  schema_version: 2,
  id: 'w-1',
  name: 'Standard',
  nodes: [{ id: 's-implement', type: 'agent', title: 'Implement Feature' }],
  edges: [],
};

/** Answers for one key and throws for any other: the id the inspector
 *  subscribes with is invisible from the rendered output, so a store that
 *  answered every key would assert against a default instead of an answer
 *  (AGENTS.md §7). */
const STREAM: AgentStreamStore = {
  subscribe: () => () => {},
  read: (stepExecutionId) => {
    if (stepExecutionId !== 'se-1') throw new Error(`unexpected stream key: ${stepExecutionId}`);
    return 'agent said this';
  },
  isTruncated: () => false,
};

/** A store that both answers and wakes, so a subscriber can be observed by the
 *  one thing the rendered output never shows: whether there is one. */
function emittingStream() {
  const woken = new Set<() => void>();
  let text = '';
  const store: AgentStreamStore = {
    subscribe: (stepExecutionId, onChange) => {
      if (stepExecutionId !== 'se-1') throw new Error(`unexpected stream key: ${stepExecutionId}`);
      woken.add(onChange);
      return () => {
        woken.delete(onChange);
      };
    },
    read: (stepExecutionId) => {
      if (stepExecutionId !== 'se-1') throw new Error(`unexpected stream key: ${stepExecutionId}`);
      return text;
    },
    isTruncated: () => false,
  };
  return {
    store,
    subscriberCount: () => woken.size,
    emit(chunk: string) {
      text += chunk;
      act(() => {
        for (const onChange of [...woken]) onChange();
      });
    },
  };
}

const RUN_EVENTS: RunEvent[] = [
  { offset: 0, run_id: 'r-1', kind: 'step_progress', payload_json: '"implement"', created_at: 0 },
];

function mount(
  target: InspectorTarget,
  graphDef: WorkflowDefinitionV2 | null = GRAPH,
  streamStore: AgentStreamStore = STREAM,
) {
  return render(
    <StepInspector
      featureId="f-1"
      target={target}
      graphDef={graphDef}
      statusByNode={{ 's-implement': { status: 'failed', errorClass: 'environment' } }}
      runEvents={RUN_EVENTS}
      streamStore={streamStore}
      onDeselect={() => {}}
      onOpenEditorForPath={() => {}}
      onOpenArtifact={() => {}}
      onRetry={() => {}}
      onReplay={() => {}}
      onStop={() => {}}
      onDecideGate={() => {}}
    />,
  );
}

afterEach(() => {
  cleanup();
  invoke.mockReset();
  feed.renders = 0;
});

describe('StepInspector — nothing to inspect', () => {
  it('says the run has no steps yet', () => {
    mount({ kind: 'empty', reason: 'no-steps' });
    expect(screen.getByText('No steps yet')).toBeInTheDocument();
    expect(screen.getByTestId('inspector-empty')).toHaveTextContent(/not been decomposed/i);
  });

  it('invites a selection when the run has steps and none is picked', () => {
    mount({ kind: 'empty', reason: 'no-selection' });
    expect(screen.getByText('No step selected')).toBeInTheDocument();
    expect(screen.getByTestId('inspector-empty')).toHaveTextContent(/pick a step/i);
  });

  it('explains a selection the run no longer has', () => {
    mount({ kind: 'empty', reason: 'stale-selection' });
    expect(screen.getByText('That step is gone')).toBeInTheDocument();
    expect(screen.getByTestId('inspector-empty')).toHaveTextContent(/no longer part of the run/i);
  });

  it('gives each reason its own words', () => {
    const wording = (reason: 'no-steps' | 'no-selection' | 'stale-selection') => {
      const { unmount } = mount({ kind: 'empty', reason });
      const text = screen.getByTestId('inspector-empty').textContent ?? '';
      unmount();
      return text;
    };
    const all = [wording('no-steps'), wording('no-selection'), wording('stale-selection')];
    expect(new Set(all).size).toBe(3);
  });
});

describe('StepInspector — a step', () => {
  it('titles itself from the workflow node', async () => {
    invoke.mockResolvedValue([]);
    mount({ kind: 'step', step: step(), blockedBy: null });
    expect(await screen.findByText('Implement Feature')).toBeInTheDocument();
  });

  it('opens on a run with no workflow definition at all', async () => {
    invoke.mockResolvedValue([]);
    mount({ kind: 'step', step: step(), blockedBy: null }, null);
    // The legacy path: no graph to take a title from, so the step id is read
    // as one rather than the pane refusing to open.
    expect(await screen.findByText('Implement')).toBeInTheDocument();
  });

  it('reads the live stream for the step it is showing', async () => {
    invoke.mockResolvedValue([]);
    const user = (await import('@testing-library/user-event')).default;
    mount({ kind: 'step', step: step({ status: 'running' }), blockedBy: null });
    await user.click(await screen.findByRole('tab', { name: 'Live' }));
    expect(screen.getByTestId('inspector')).toHaveTextContent('agent said this');
  });

  /**
   * The regression this exists to catch is one of *reach*, not of behaviour:
   * every earlier arrangement of the subscription renders identically, and the
   * inspector opens on the running step with no user action, so a subscription
   * anywhere above `LiveTab` runs for the whole of every run.
   */
  it('leaves the Overview tab untouched while an agent streams', async () => {
    invoke.mockResolvedValue([]);
    const user = (await import('@testing-library/user-event')).default;
    const stream = emittingStream();
    mount({ kind: 'step', step: step({ status: 'running' }), blockedBy: null }, GRAPH, stream.store);
    await screen.findByText('Implement Feature');
    await waitFor(() => expect(feed.renders).toBeGreaterThan(0));
    await act(async () => {});

    const before = feed.renders;
    for (let i = 0; i < 5; i += 1) stream.emit(`chunk ${i}\n`);
    expect(feed.renders).toBe(before);
    expect(stream.subscriberCount()).toBe(0);

    // Not a store nobody could have read: the same chunks are on screen the
    // moment the one tab that asks for them is opened.
    await user.click(screen.getByRole('tab', { name: 'Live' }));
    expect(stream.subscriberCount()).toBe(1);
    expect(screen.getByTestId('inspector')).toHaveTextContent('chunk 4');
  });

  it('offers retry on a failure and neither stop nor gate', async () => {
    invoke.mockResolvedValue([]);
    const user = (await import('@testing-library/user-event')).default;
    mount({ kind: 'step', step: step({ status: 'failed' }), blockedBy: null });
    await user.click(await screen.findByRole('tab', { name: 'Actions' }));
    expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /stop/i })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Decide' })).not.toBeInTheDocument();
  });

  it('offers the gate decision on a waiting gate', async () => {
    invoke.mockResolvedValue([]);
    const user = (await import('@testing-library/user-event')).default;
    mount(
      { kind: 'step', step: step({ status: 'awaiting_gate', step_id: 's-implement' }), blockedBy: null },
      { ...GRAPH, nodes: [{ id: 's-implement', type: 'gate', title: 'Review Gate' }] },
    );
    await user.click(await screen.findByRole('tab', { name: 'Actions' }));
    expect(screen.getByRole('button', { name: 'Decide' })).toBeInTheDocument();
  });
});
