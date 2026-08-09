/**
 * No card re-renders while an agent streams, and moving the selection re-renders
 * only the two rows whose selected state changed.
 *
 * The original defect was `setStreamContent({ ...bufferRef.current })` once per
 * animation frame reaching every `StepCard`; Phase 1 answered it with a per-step
 * subscription plus `memo`, and Phase 3 left the stream a single mount site in
 * the inspector, so the assertion is now zero card renders rather than "only the
 * streaming one".
 *
 * **What actually fails this file is an unstable card prop** — a fresh object,
 * array or closure passed to a memoized row — which is what both describes below
 * detect and why they count renders of the real subtree, via a counting
 * `StepMetrics` (the one child every card renders unconditionally). Asserting
 * that `memo` is present passes even when a prop defeats it.
 *
 * What this file cannot see: a subscription re-introduced *inside*
 * `StepTimeline`. `memo` absorbs it, so no card renders and every claim here
 * still holds. The structural version of that claim — nothing around the
 * inspector wakes on a chunk — is `FeatureDetail.liveStream.test.tsx`'s, which
 * mounts the whole detail view for exactly that reason.
 */
import { act, render, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useCallback, useRef, useState } from 'react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import { loadAgentCatalog } from '../../lib/agentCatalog';
import { DEFAULT_DENSITY } from '../../lib/density';
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
vi.mock('./StepMetrics', () => ({
  StepMetrics: ({ step }: { step: StepExecution }) => {
    cardRenders[step.id] = (cardRenders[step.id] ?? 0) + 1;
    return null;
  },
}));

import { StepCard } from './StepCard';
import { StepTimeline } from './StepTimeline';
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

/**
 * `FeatureDetail`'s wiring of the timeline, reproduced rather than approximated.
 *
 * The stream store is built from the real `useAgentStream` so the chunks below
 * travel the path the app's do — the timeline no longer takes the store as a
 * prop, and mounting the hook here is what proves that a stream arriving at this
 * subtree reaches no card. `onSelect` is `useCallback`-stable exactly as
 * `useStepSelection`'s is.
 */
function Harness() {
  useAgentStream(FEATURE_ID);
  const stepCardRefs = useRef<Record<string, HTMLDivElement | null>>({});
  const [selectedStepId, setSelectedStepId] = useState<string | null>(null);
  const onSelect = useCallback((id: string) => setSelectedStepId(id), []);

  return (
    <StepTimeline
      steps={STEPS}
      remoteRun={null}
      remoteMachineName={null}
      hasBootstrapPhases={false}
      gateStepExecutionId={null}
      stepCardRefs={stepCardRefs}
      selectedStepId={selectedStepId}
      density={DEFAULT_DENSITY}
      onSelect={onSelect}
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
  it('re-renders no card at all', async () => {
    render(<Harness />);
    await waitFor(() => expect(handlers.agent_stream?.length).toBeGreaterThan(0));

    const before = { ...cardRenders };
    // Otherwise "nobody re-rendered" would also hold for a timeline that
    // rendered no cards at all.
    expect(Object.keys(before).sort()).toEqual([...IDLE_IDS, STREAMING_ID].sort());

    for (let i = 0; i < 10; i += 1) {
      emitChunk(STREAMING_ID, `chunk ${i}\n`);
      await flushFrame();
    }

    for (const id of [...IDLE_IDS, STREAMING_ID]) {
      expect(cardRenders[id] - (before[id] ?? 0)).toBe(0);
    }
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
