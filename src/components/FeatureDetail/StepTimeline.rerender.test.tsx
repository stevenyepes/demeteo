/**
 * A streaming agent may only re-render its own card, and the text it streams
 * may not grow without bound.
 *
 * The defect these cover: the stream buffer was flushed into state as
 * `setStreamContent({ ...bufferRef.current })` once per animation frame, and
 * that record reached every `StepCard` — so a single agent's chunks re-rendered
 * the whole run at frame rate, including the cards that read nothing from it.
 * Counting renders of the real subtree (via a counting `StepArtifactList`, the
 * one child every card renders unconditionally) is the point: asserting that
 * `memo` is present passes even when an unstable prop defeats it.
 */
import { act, render, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useCallback, useRef } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { STREAM_CAP_CHARS } from '../../lib/streamBuffer';
import type { StepExecution } from '../../types';

const handlers: Record<string, Array<(e: { payload: unknown }) => void>> = {};
vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => {
    (handlers[event] ??= []).push(cb);
    return Promise.resolve(() => {
      handlers[event] = (handlers[event] ?? []).filter((h) => h !== cb);
    });
  },
}));

const cardRenders: Record<string, number> = {};
vi.mock('./StepArtifactList', () => ({
  StepArtifactList: ({ step }: { step: StepExecution }) => {
    cardRenders[step.id] = (cardRenders[step.id] ?? 0) + 1;
    return null;
  },
}));

import { StepCard } from './StepCard';
import { StepTimeline } from './StepTimeline';
import type { HarnessOverrides } from './useHarnessOverrides';
import { useAgentStream } from './useAgentStream';

const FEATURE_ID = 'f-1';
const STREAMING_ID = 'se-3';

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
  step({ id: 'se-2', step_id: 's-plan', step_index: 1 }),
  step({ id: STREAMING_ID, step_id: 's-implement', step_index: 2, status: 'running' }),
  step({ id: 'se-4', step_id: 's-review', step_index: 3, status: 'pending' }),
  step({ id: 'se-5', step_id: 's-validate', step_index: 4, status: 'pending' }),
];

const IDLE_IDS = ['se-1', 'se-2', 'se-4', 'se-5'] as const;

const noop = () => {};

const OVERRIDES: HarnessOverrides = {
  availableModels: [],
  selectedModel: '',
  setSelectedModel: noop,
  isLoadingModels: false,
  availableAgents: [],
  selectedAgent: '',
  selectedEffort: '',
  setSelectedEffort: noop,
  featureAgentKind: 'opencode',
  retryEffortLevels: [],
  onAgentChange: noop,
  adoptFeatureModel: noop,
  probeForFeature: noop,
};

/** `FeatureDetail`'s wiring of the timeline, with everything the cards do not
 *  read held constant so a re-render can only come from the stream. */
function Harness() {
  const stream = useAgentStream(FEATURE_ID);
  const stepCardRefs = useRef<Record<string, HTMLDivElement | null>>({});
  const toggleStream = useCallback(
    (id: string) => stream.setActiveStreamId((prev) => (prev === id ? null : id)),
    [stream.setActiveStreamId],
  );

  return (
    <StepTimeline
      steps={STEPS}
      remoteRun={null}
      remoteMachineName={null}
      hasBootstrapPhases={false}
      gateStepExecutionId={null}
      stepCardRefs={stepCardRefs}
      harnessBaseline={null}
      overrides={OVERRIDES}
      selectedArtifactPath={null}
      activeStreamId={stream.activeStreamId}
      streamStore={stream.store}
      onToggleStream={toggleStream}
      onOpenArtifact={noop}
      onStartReplay={noop}
      onRetry={noop}
      onStop={noop}
      onDecideGate={noop}
    />
  );
}

function emitChunk(stepExecutionId: string, content: string) {
  for (const handler of handlers.agent_stream ?? []) {
    handler({ payload: { feature_id: FEATURE_ID, step_execution_id: stepExecutionId, content } });
  }
}

/** Lets the buffer's own animation frame run: a frame scheduled here is queued
 *  behind the flush the chunk above scheduled. */
async function flushFrame() {
  await act(async () => {
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() => resolve());
    });
  });
}

function findButton(container: HTMLElement, label: string): HTMLButtonElement {
  const button = [...container.querySelectorAll('button')].find((b) =>
    (b.textContent ?? '').includes(label),
  );
  if (!button) throw new Error(`no button matching "${label}"`);
  return button;
}

async function mountWithStreamOpen() {
  const { container } = render(<Harness />);
  await waitFor(() => expect(handlers.agent_stream?.length).toBeGreaterThan(0));
  await userEvent.click(findButton(container, 'View Agent Reasoning'));
  return container;
}

function streamText(container: HTMLElement): string {
  const pre = container.querySelector('pre');
  if (!pre) throw new Error('the live stream block is not rendered');
  return pre.textContent ?? '';
}

afterEach(() => {
  for (const key of Object.keys(handlers)) delete handlers[key];
  for (const key of Object.keys(cardRenders)) delete cardRenders[key];
});

describe('StepTimeline under a live agent stream', () => {
  it('re-renders only the streaming step’s card', async () => {
    const container = await mountWithStreamOpen();

    const before = { ...cardRenders };
    // Otherwise "nobody else re-rendered" would also hold for a timeline that
    // rendered no other cards at all.
    expect(Object.keys(before).sort()).toEqual([...IDLE_IDS, STREAMING_ID].sort());

    for (let i = 0; i < 10; i += 1) {
      emitChunk(STREAMING_ID, `chunk ${i}\n`);
      await flushFrame();
    }

    // The stream really did arrive — otherwise "nobody re-rendered" would be
    // satisfied by a timeline that never received a chunk at all.
    expect(streamText(container)).toContain('chunk 9');
    expect(cardRenders[STREAMING_ID]).toBeGreaterThan(before[STREAMING_ID] ?? 0);

    for (const id of IDLE_IDS) {
      expect(cardRenders[id] - (before[id] ?? 0)).toBe(0);
    }
  });

  it('keeps the retained stream inside the buffer cap and says so', async () => {
    const container = await mountWithStreamOpen();

    const chunk = `${'x'.repeat(64 * 1024)}\n`;
    for (let i = 0; i < 8; i += 1) {
      emitChunk(STREAMING_ID, chunk);
      await flushFrame();
    }

    const retained = streamText(container);
    expect(retained.length).toBeLessThanOrEqual(STREAM_CAP_CHARS);
    expect(retained.length).toBeGreaterThan(STREAM_CAP_CHARS / 2);
    expect(container.textContent).toContain('Earlier output dropped');
  });

  it('keeps StepCard memoized', () => {
    expect(StepCard).toHaveProperty('$$typeof', Symbol.for('react.memo'));
  });
});
