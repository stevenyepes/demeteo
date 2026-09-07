import { useState } from 'react';

import { EVENT_ASK_TURN_STATUS, type AskTurnStatusPayload } from '../lib/ask';
import { phaseOfStatus } from '../lib/askActivity';
import { useTauriEvent } from './useTauriEvent';

/**
 * The ids of Ask threads with a turn running *right now*.
 *
 * Liveness is read off `ask_turn_status` and never stored: `AskThread` only
 * knows what the last *settled* turn left behind (`turn_count`,
 * `updated_at`), so nothing a list fetch returns can say a thread is mid-turn.
 * `DiscoverySection.tsx` faces the identical gap for Discovery cards and
 * answers it the same way.
 *
 * **A mount starts empty and stays that way until the next event.** That is a
 * property of the source, not a bug to paper over with a fetch: a thread that
 * began its turn before this hook mounted is invisible to it, so the only
 * honest rendering of an unlisted thread is "not known to be running" — which
 * is why callers use it to *add* a pulse rather than to gate anything.
 */
export function useLiveAskTurns(): ReadonlySet<string> {
  const [running, setRunning] = useState<Set<string>>(new Set());

  useTauriEvent<AskTurnStatusPayload>(EVENT_ASK_TURN_STATUS, ({ thread_id, status }) => {
    setRunning((prev) => {
      const next = new Set(prev);
      if (phaseOfStatus(status) !== null) next.add(thread_id);
      else next.delete(thread_id);
      return next;
    });
  });

  return running;
}
