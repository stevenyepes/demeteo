import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from 'react';

import { appendCapped } from '../../lib/streamBuffer';
import {
  EVENT_DISCOVERY_AGENT_EVENT,
  isTextDelta,
  type DiscoveryAgentEventPayload,
} from '../../lib/discovery';
import { useTauriEvent } from '../../hooks/useTauriEvent';

/**
 * Read side of the interviewer's live turn — the same shape as
 * `FeatureDetail/useAgentStream.ts`, and for the same reason.
 *
 * Deltas arrive far faster than a frame, so publishing them through state
 * re-renders whatever the subscriber's parent also renders at frame rate. Here
 * that parent holds the whole transcript and, one column over, the ticket
 * graph. The wake is therefore coalesced to one animation frame and delivered
 * only to the leaf that asked for this discovery's text — `StreamingBubble`,
 * which is mounted only while a turn runs.
 */
export interface DiscoveryStreamStore {
  subscribe: (discoveryId: string, onChange: () => void) => () => void;
  read: (discoveryId: string) => string;
}

const noop = () => {};

export function useDiscoveryStream(): {
  store: DiscoveryStreamStore;
  /** Drop a discovery's buffer — its stored message is the transcript now. */
  reset: (discoveryId: string) => void;
} {
  const buffers = useRef(new Map<string, string>());
  const listeners = useRef(new Map<string, Set<() => void>>());
  const dirty = useRef(new Set<string>());
  const frame = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (frame.current !== null) cancelAnimationFrame(frame.current);
    },
    [],
  );

  const wake = useCallback((discoveryId: string) => {
    dirty.current.add(discoveryId);
    if (frame.current !== null) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = null;
      const woken = [...dirty.current];
      dirty.current.clear();
      for (const id of woken) {
        for (const onChange of [...(listeners.current.get(id) ?? [])]) onChange();
      }
    });
  }, []);

  useTauriEvent<DiscoveryAgentEventPayload>(
    EVENT_DISCOVERY_AGENT_EVENT,
    ({ discovery_id, event }) => {
      if (!isTextDelta(event)) return;
      const prev = buffers.current.get(discovery_id) ?? '';
      const next = appendCapped(prev, event.delta);
      if (next === prev) return;
      buffers.current.set(discovery_id, next);
      wake(discovery_id);
    },
  );

  const reset = useCallback(
    (discoveryId: string) => {
      if (!buffers.current.has(discoveryId)) return;
      buffers.current.delete(discoveryId);
      wake(discoveryId);
    },
    [wake],
  );

  const store = useMemo<DiscoveryStreamStore>(
    () => ({
      subscribe: (discoveryId, onChange) => {
        const subscribers = listeners.current.get(discoveryId) ?? new Set<() => void>();
        listeners.current.set(discoveryId, subscribers);
        subscribers.add(onChange);
        return () => {
          subscribers.delete(onChange);
          if (subscribers.size === 0 && listeners.current.get(discoveryId) === subscribers) {
            listeners.current.delete(discoveryId);
          }
        };
      },
      read: (discoveryId) => buffers.current.get(discoveryId) ?? '',
    }),
    [],
  );

  return { store, reset };
}

/** One discovery's partial turn. Call this at the leaf that renders it. */
export function useStreamedTurn(store: DiscoveryStreamStore, discoveryId: string | null): string {
  const subscribe = useCallback(
    (onChange: () => void) => (discoveryId === null ? noop : store.subscribe(discoveryId, onChange)),
    [store, discoveryId],
  );
  const snapshot = useCallback(
    () => (discoveryId === null ? '' : store.read(discoveryId)),
    [store, discoveryId],
  );
  return useSyncExternalStore(subscribe, snapshot);
}
