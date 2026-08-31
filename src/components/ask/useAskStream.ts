import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from 'react';

import { foldTurnEvent, openTurn, NO_TURN, type LiveTurn, type TurnPhase } from '../../lib/askActivity';
import { EVENT_ASK_AGENT_EVENT, type AskAgentEventPayload } from '../../lib/ask';
import { useTauriEvent } from '../../hooks/useTauriEvent';

/**
 * Read side of an Ask thread's live turn — mirrors
 * `discovery/useDiscoveryStream.ts` exactly, keyed by `thread_id` instead of
 * `discovery_id`.
 *
 * Deltas arrive far faster than a frame, so publishing them through state
 * re-renders whatever the subscriber's parent also renders at frame rate. Here
 * that parent holds the whole transcript and, one column over, the canvas
 * pane. The wake is therefore coalesced to one animation frame and delivered
 * only to the leaf that asked for this thread's turn — `AskStreamingBubble` /
 * `AskCanvasPane`, mounted only while a turn runs.
 *
 * What a snapshot holds is everything the bubble draws, in one object: the
 * text, the calls in flight, the capped ledger and the counters behind the
 * summary. One object rather than four slices because they all change on the
 * same events and would otherwise be four subscriptions waking the same leaf.
 * `useSyncExternalStore` compares with `Object.is`, so the object is replaced
 * on change and handed back by identity otherwise.
 */
export interface AskStreamStore {
  subscribe: (threadId: string, onChange: () => void) => () => void;
  read: (threadId: string) => LiveTurn;
}

const noop = () => {};

export function useAskStream(): {
  store: AskStreamStore;
  /** Open a turn in `phase`, stamping its start the first time. Idempotent on
   *  the stamp: the click that sends, the `setting_up` status behind it and the
   *  `running` status after are one turn, and restamping would restart the
   *  clock the user is already watching. The phase moves on each of them. */
  begin: (threadId: string, phase: TurnPhase) => void;
  /** Drop a thread's turn — its stored message is the transcript now. */
  end: (threadId: string) => void;
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

  const wake = useCallback((threadId: string) => {
    dirty.current.add(threadId);
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

  useTauriEvent<AskAgentEventPayload>(EVENT_ASK_AGENT_EVENT, ({ thread_id, event }) => {
    // An event before `begin` means the turn was started somewhere this
    // surface did not see — a reload mid-turn. It still has a start, and now
    // is the closest one that is true.
    const prev = turns.current.get(thread_id) ?? openTurn(Date.now(), 'working');
    const next = foldTurnEvent(prev, event);
    if (next === prev && turns.current.has(thread_id)) return;
    turns.current.set(thread_id, next);
    wake(thread_id);
  });

  const begin = useCallback(
    (threadId: string, phase: TurnPhase) => {
      const open = turns.current.get(threadId);
      if (open?.phase === phase) return;
      turns.current.set(threadId, open === undefined ? openTurn(Date.now(), phase) : { ...open, phase });
      wake(threadId);
    },
    [wake],
  );

  const end = useCallback(
    (threadId: string) => {
      if (!turns.current.has(threadId)) return;
      turns.current.delete(threadId);
      wake(threadId);
    },
    [wake],
  );

  const store = useMemo<AskStreamStore>(
    () => ({
      subscribe: (threadId, onChange) => {
        const subscribers = listeners.current.get(threadId) ?? new Set<() => void>();
        listeners.current.set(threadId, subscribers);
        subscribers.add(onChange);
        return () => {
          subscribers.delete(onChange);
          if (subscribers.size === 0 && listeners.current.get(threadId) === subscribers) {
            listeners.current.delete(threadId);
          }
        };
      },
      read: (threadId) => turns.current.get(threadId) ?? NO_TURN,
    }),
    [],
  );

  return { store, begin, end };
}

/** One thread's partial turn. Call this at the leaf that renders it. */
export function useStreamedTurn(store: AskStreamStore, threadId: string | null): LiveTurn {
  const subscribe = useCallback(
    (onChange: () => void) => (threadId === null ? noop : store.subscribe(threadId, onChange)),
    [store, threadId],
  );
  const snapshot = useCallback(() => (threadId === null ? NO_TURN : store.read(threadId)), [store, threadId]);
  return useSyncExternalStore(subscribe, snapshot);
}
