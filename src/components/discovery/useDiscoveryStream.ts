import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from 'react';

import {
  foldTurnEvent,
  openTurn,
  NO_TURN,
  type LiveTurn,
  type TurnPhase,
} from '../../lib/discoveryActivity';
import { EVENT_DISCOVERY_AGENT_EVENT, type DiscoveryAgentEventPayload } from '../../lib/discovery';
import { useTauriEvent } from '../../hooks/useTauriEvent';

/**
 * Read side of the interviewer's live turn — the same shape as
 * `FeatureDetail/useAgentStream.ts`, and for the same reason.
 *
 * Deltas arrive far faster than a frame, so publishing them through state
 * re-renders whatever the subscriber's parent also renders at frame rate. Here
 * that parent holds the whole transcript and, one column over, the ticket
 * graph. The wake is therefore coalesced to one animation frame and delivered
 * only to the leaf that asked for this discovery's turn — `StreamingBubble`,
 * which is mounted only while a turn runs.
 *
 * What a snapshot holds is everything the bubble draws, in one object: the
 * text, the calls in flight, the capped ledger and the counters behind the
 * summary. One object rather than four slices because they all change on the
 * same events and would otherwise be four subscriptions waking the same leaf.
 * `useSyncExternalStore` compares with `Object.is`, so the object is replaced
 * on change and handed back by identity otherwise.
 */
export interface DiscoveryStreamStore {
  subscribe: (discoveryId: string, onChange: () => void) => () => void;
  read: (discoveryId: string) => LiveTurn;
}

const noop = () => {};

export function useDiscoveryStream(): {
  store: DiscoveryStreamStore;
  /** Open a turn in `phase`, stamping its start the first time. Idempotent on
   *  the stamp: the click that sends, the `setting_up` status behind it and the
   *  `running` status after are one turn, and restamping would restart the
   *  clock the user is already watching. The phase moves on each of them. */
  begin: (discoveryId: string, phase: TurnPhase) => void;
  /** Drop a discovery's turn — its stored message is the transcript now. */
  end: (discoveryId: string) => void;
} {
  const turns = useRef(new Map<string, LiveTurn>());
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
      // An event before `begin` means the turn was started somewhere this
      // surface did not see — a decomposition pass, or a reload mid-turn. It
      // still has a start, and now is the closest one that is true.
      const prev = turns.current.get(discovery_id) ?? openTurn(Date.now(), 'working');
      const next = foldTurnEvent(prev, event);
      if (next === prev && turns.current.has(discovery_id)) return;
      turns.current.set(discovery_id, next);
      wake(discovery_id);
    },
  );

  const begin = useCallback(
    (discoveryId: string, phase: TurnPhase) => {
      const open = turns.current.get(discoveryId);
      if (open?.phase === phase) return;
      turns.current.set(
        discoveryId,
        open === undefined ? openTurn(Date.now(), phase) : { ...open, phase },
      );
      wake(discoveryId);
    },
    [wake],
  );

  const end = useCallback(
    (discoveryId: string) => {
      if (!turns.current.has(discoveryId)) return;
      turns.current.delete(discoveryId);
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
      read: (discoveryId) => turns.current.get(discoveryId) ?? NO_TURN,
    }),
    [],
  );

  return { store, begin, end };
}

/** One discovery's partial turn. Call this at the leaf that renders it. */
export function useStreamedTurn(store: DiscoveryStreamStore, discoveryId: string | null): LiveTurn {
  const subscribe = useCallback(
    (onChange: () => void) => (discoveryId === null ? noop : store.subscribe(discoveryId, onChange)),
    [store, discoveryId],
  );
  const snapshot = useCallback(
    () => (discoveryId === null ? NO_TURN : store.read(discoveryId)),
    [store, discoveryId],
  );
  return useSyncExternalStore(subscribe, snapshot);
}
