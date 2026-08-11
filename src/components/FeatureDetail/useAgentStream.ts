import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import { appendCapped, wasTruncated } from '../../lib/streamBuffer';
import { useTauriEvent } from '../../hooks/useTauriEvent';

/**
 * Read side of the live agent stream: one step's text, one subscription.
 *
 * Publishing the whole `Record<stepExecutionId, string>` through state is what
 * made every card in the run re-render at frame rate — the flush copies the
 * record, so its identity changes on every frame and reaches cards that read
 * nothing from it. A per-step subscription wakes only the consumers that asked
 * for the step that moved.
 */
export interface AgentStreamStore {
  subscribe: (stepExecutionId: string, onChange: () => void) => () => void;
  read: (stepExecutionId: string) => string;
  isTruncated: (stepExecutionId: string) => boolean;
}

const noop = () => {};

/**
 * `useSyncExternalStore` compares snapshots with `Object.is`, so a selector may
 * only return a primitive: an object assembled per read is a new identity every
 * time and re-renders forever.
 */
function useStreamSlice<T extends string | boolean>(
  store: AgentStreamStore,
  stepExecutionId: string | null,
  select: (store: AgentStreamStore, stepExecutionId: string) => T,
  whenUnsubscribed: T,
): T {
  const subscribe = useCallback(
    (onChange: () => void) =>
      stepExecutionId === null ? noop : store.subscribe(stepExecutionId, onChange),
    [store, stepExecutionId],
  );
  const snapshot = useCallback(
    () => (stepExecutionId === null ? whenUnsubscribed : select(store, stepExecutionId)),
    [store, stepExecutionId, select, whenUnsubscribed],
  );
  return useSyncExternalStore(subscribe, snapshot);
}

const selectText = (store: AgentStreamStore, stepExecutionId: string) => store.read(stepExecutionId);
const selectTruncated = (store: AgentStreamStore, stepExecutionId: string) =>
  store.isTruncated(stepExecutionId);

/** One step's buffered output. A `null` step subscribes to nothing. */
export function useStreamText(store: AgentStreamStore, stepExecutionId: string | null): string {
  return useStreamSlice(store, stepExecutionId, selectText, '');
}

/** Whether that buffer is a tail rather than the whole turn. */
export function useStreamTruncated(store: AgentStreamStore, stepExecutionId: string | null): boolean {
  return useStreamSlice(store, stepExecutionId, selectTruncated, false);
}

/**
 * The buffers only. Which step's output is on screen is not this hook's
 * business any more: it used to hold an `activeStreamId` because each timeline
 * card had a stream to open and close, and Phase 3 left the stream one mount
 * site driven by the inspector's selection (UI_REDESIGN_PLAN §3.1). A second
 * piece of "which step is showing" state would be free to disagree with that
 * selection.
 */
export function useAgentStream(featureId: string) {
  const buffers = useRef(new Map<string, string>());
  const truncated = useRef(new Set<string>());
  const listeners = useRef(new Map<string, Set<() => void>>());
  // Chunks arrive far faster than a frame; the wake is coalesced to one per
  // animation frame, and only for the steps that actually moved within it.
  const dirty = useRef(new Set<string>());
  const frameRef = useRef<number | null>(null);
  useEffect(() => () => {
    if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
  }, []);

  useTauriEvent<{ feature_id: string; step_execution_id: string; content: string }>('agent_stream', ({ feature_id, step_execution_id, content }) => {
    if (feature_id !== featureId) return;
    const prev = buffers.current.get(step_execution_id) ?? '';
    const next = appendCapped(prev, content);
    // Truncation is its own reason to wake: an append can drop exactly as much
    // as it adds — a repeated chunk against a full buffer — leaving the text
    // identical while the buffer stops being the whole turn.
    const newlyTruncated =
      wasTruncated(prev, content, next) && !truncated.current.has(step_execution_id);
    if (newlyTruncated) truncated.current.add(step_execution_id);
    if (next === prev && !newlyTruncated) return;
    buffers.current.set(step_execution_id, next);
    dirty.current.add(step_execution_id);
    if (frameRef.current !== null) return;
    frameRef.current = requestAnimationFrame(() => {
      frameRef.current = null;
      const woken = [...dirty.current];
      dirty.current.clear();
      for (const id of woken) {
        for (const onChange of [...(listeners.current.get(id) ?? [])]) onChange();
      }
    });
  });

  const store = useMemo<AgentStreamStore>(() => ({
    subscribe: (stepExecutionId, onChange) => {
      const subscribers = listeners.current.get(stepExecutionId) ?? new Set<() => void>();
      listeners.current.set(stepExecutionId, subscribers);
      subscribers.add(onChange);
      return () => {
        subscribers.delete(onChange);
        // Identity-checked: a late unsubscribe must not evict the set a
        // subscriber that arrived after it is reading.
        if (subscribers.size === 0 && listeners.current.get(stepExecutionId) === subscribers) {
          listeners.current.delete(stepExecutionId);
        }
      };
    },
    read: (stepExecutionId) => buffers.current.get(stepExecutionId) ?? '',
    isTruncated: (stepExecutionId) => truncated.current.has(stepExecutionId),
  }), []);

  return { store };
}
