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
import { useCallback, useRef, useState } from 'react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import { loadAgentCatalog } from '../../lib/agentCatalog';
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
import { useAgentStream } from './useAgentStream';
import { useHarnessOverrides } from './useHarnessOverrides';
import { useRerunActions } from './useRerunActions';

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

/**
 * `FeatureDetail`'s wiring of the timeline, reproduced rather than approximated.
 *
 * The three props the cards get from a hook — `overrides`, `onRetry`, `onStop` —
 * are taken from the real hooks, and the two inputs `FeatureDetailView` rebuilds
 * on every render (`reload`, `refreshRemoteRun`) are passed as fresh literals
 * here for the same reason. Held constant instead, this file asserted against a
 * wiring the app does not have: a hook returning a new object literal per render
 * defeats every `memo` below and every claim here still passed.
 *
 * `onSelect` is `useCallback`-stable exactly as `useStepSelection`'s is.
 */
function Harness() {
  const stream = useAgentStream(FEATURE_ID);
  const stepCardRefs = useRef<Record<string, HTMLDivElement | null>>({});
  const [selectedStepId, setSelectedStepId] = useState<string | null>(null);
  const toggleStream = useCallback(
    (id: string) => stream.setActiveStreamId((prev) => (prev === id ? null : id)),
    [stream.setActiveStreamId],
  );
  const onSelect = useCallback((id: string) => setSelectedStepId(id), []);

  const overrides = useHarnessOverrides();
  const rerun = useRerunActions({
    featureId: FEATURE_ID,
    remoteRun: null,
    refreshRemoteRun: () => {},
    reload: () => {},
    setFeatureStatus: () => {},
    overrides,
  });

  return (
    <StepTimeline
      steps={STEPS}
      remoteRun={null}
      remoteMachineName={null}
      hasBootstrapPhases={false}
      gateStepExecutionId={null}
      stepCardRefs={stepCardRefs}
      harnessBaseline={null}
      overrides={overrides}
      selectedArtifactPath={null}
      selectedStepId={selectedStepId}
      activeStreamId={stream.activeStreamId}
      streamStore={stream.store}
      onSelect={onSelect}
      onToggleStream={toggleStream}
      onOpenArtifact={noop}
      onStartReplay={noop}
      onRetry={rerun.handleRetryStep}
      onStop={rerun.handleStopStep}
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

/** `useAgentCatalog` seeds itself from this module-level cache synchronously, so
 *  priming it here is what keeps the catalog fetch from landing mid-test as a
 *  re-render nobody asked for — and makes the counts below the harness's own. */
beforeAll(async () => {
  vi.mocked(invoke).mockImplementation((async (cmd: string) =>
    cmd === 'list_agents' ? [] : undefined) as unknown as typeof invoke);
  await loadAgentCatalog();
});

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

describe('StepTimeline selection', () => {
  it('re-renders only the rows whose selected state changed', async () => {
    const { container } = render(<Harness />);
    await waitFor(() => expect(handlers.agent_stream?.length).toBeGreaterThan(0));

    const rows = [...container.querySelectorAll('[data-step-row]')] as HTMLButtonElement[];
    expect(rows).toHaveLength(STEPS.length);

    await userEvent.click(rows[0]);
    const afterFirst = { ...cardRenders };
    await userEvent.click(rows[2]);

    // Two cards change: the one losing the selection and the one taking it.
    // Everything else must sit still — a fresh `onSelect` identity, or a
    // selection prop derived per render, re-renders the whole run instead.
    for (const [id, renders] of Object.entries(cardRenders)) {
      const moved = renders - (afterFirst[id] ?? 0);
      expect(moved).toBe(id === 'se-1' || id === STREAMING_ID ? 1 : 0);
    }
  });

  it('marks exactly the selected row', async () => {
    const { container } = render(<Harness />);
    await waitFor(() => expect(handlers.agent_stream?.length).toBeGreaterThan(0));

    const rows = () => [...container.querySelectorAll('[data-step-row]')] as HTMLButtonElement[];
    expect(rows().map((r) => r.getAttribute('aria-current'))).toEqual(
      STEPS.map(() => null),
    );

    await userEvent.click(rows()[3]);
    expect(rows().map((r) => r.getAttribute('aria-current'))).toEqual(
      STEPS.map((_, i) => (i === 3 ? 'step' : null)),
    );
  });
});
