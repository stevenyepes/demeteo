import React, { useEffect, useRef, useState } from 'react';
import { Radio } from 'lucide-react';

import type { RemoteRunMirror, RunEvent } from '../../types';
import { activitySync, type ActivityTransport } from '../../lib/activitySync';
import { formatError } from '../../lib/errors';
import { streamRemoteEvents } from '../../lib/remoteRuns';
import { TONE_TEXT } from '../../lib/runStatus';
import { RunEventFeed } from '../RunEventFeed';
import { Disclosure } from '../ui/Disclosure';

/** How often the detached tail asks the runner for new rows. Read by the
 *  affordance as well as by the interval, so the two cannot drift — the caption
 *  this replaces named a different poll's interval and stayed wrong for both. */
export const REMOTE_TAIL_POLL_MS = 2_000;

/** A single blip (tunnel hiccup, one dropped poll) shouldn't paint a permanent
 *  error over an otherwise-live view — only surface one once a few consecutive
 *  polls have failed, and clear it the moment a poll succeeds. */
const FAILURE_THRESHOLD = 3;

interface ActivityPanelProps {
  /** The unified feed to render, oldest→newest. Local runs push it through
   *  `useRunEvents`; a detached run's rows arrive from the tail below and come
   *  back down through this prop, so the panel keeps no second copy. */
  events: RunEvent[];
  /** Non-null iff this run is detached — enables the tunnel tail. */
  remote: { run: RemoteRunMirror; machineName: string; onEvents: (events: RunEvent[]) => void } | null;
  /** The run has finished: the log cannot grow further. */
  terminal: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/**
 * The run's activity log — one collapsible surface for both transports
 * (`docs/UI_REDESIGN_PLAN.md` §1 **D**).
 *
 * Two things it is deliberately not: it is not per-node, so it does not belong
 * in the step inspector, where a run-level feed sat under a node-scoped tab
 * until Phase 5 moved it here; and it is not a second accumulation of the feed.
 * A detached run's rows are handed *up* (`remote.onEvents`) to the hook that
 * already caps and de-dupes them, and handed back down as `events` — the copy
 * this panel used to keep alongside that one was unbounded, which is the cap
 * rule in §4.3 applied to everything except the log most likely to be long.
 *
 * Collapsing unmounts the tail with the body, so a closed panel costs no tunnel
 * traffic. That is a real behaviour change to a user, not an implementation
 * detail — `activitySync` puts it in words next to the title, because a run
 * whose bootstrap stepper stops advancing while the panel is shut is otherwise
 * indistinguishable from a run that stopped.
 */
export function ActivityPanel({
  events,
  remote,
  terminal,
  open,
  onOpenChange,
}: ActivityPanelProps): React.ReactElement {
  const [error, setError] = useState<string>('');
  const [consecutiveFailures, setConsecutiveFailures] = useState(0);
  const offsetRef = useRef(0);
  const bottomRef = useRef<HTMLDivElement>(null);

  const transport: ActivityTransport = remote ? 'remote' : 'local';
  const machineId = remote?.run.machine_id ?? null;
  const runId = remote?.run.run_id ?? null;
  const onEvents = remote?.onEvents;

  useEffect(() => {
    // Closed panel = nothing to paint; don't keep the poll (and its SSH round
    // trips) alive for it. `offsetRef` outlives the body, so reopening resumes
    // from the last consumed row instead of refetching the whole log.
    if (!open || machineId === null || runId === null) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const fresh = await streamRemoteEvents(machineId, runId, offsetRef.current);
        if (cancelled) return;
        setError('');
        setConsecutiveFailures(0);
        if (!fresh || fresh.length === 0) return;
        offsetRef.current = Math.max(offsetRef.current, ...fresh.map((e) => e.offset));
        onEvents?.(fresh);
        setTimeout(() => bottomRef.current?.scrollIntoView({ behavior: 'smooth', block: 'nearest' }), 0);
      } catch (e) {
        if (cancelled) return;
        // A terminal run gets exactly one fetch attempt — no retry loop to
        // eventually succeed — so don't wait for a streak that will never
        // accumulate; surface the failure right away.
        if (terminal) {
          setError(formatError(e));
          return;
        }
        setConsecutiveFailures((n) => {
          const next = n + 1;
          if (next >= FAILURE_THRESHOLD) setError(formatError(e));
          return next;
        });
      }
    };
    void poll();
    if (terminal) return () => { cancelled = true; };
    const interval = setInterval(() => void poll(), REMOTE_TAIL_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [machineId, runId, terminal, open, onEvents]);

  const sync = activitySync({
    transport,
    open,
    terminal,
    pollMs: REMOTE_TAIL_POLL_MS,
    consecutiveFailures,
    errored: Boolean(error),
  });

  const machineName = remote?.machineName ?? null;

  return (
    <Disclosure
      title="Activity"
      open={open}
      onOpenChange={onOpenChange}
      icon={<Radio className={`w-4 h-4 ${TONE_TEXT[sync.tone]}`} aria-hidden="true" />}
      meta={
        <>
          {/* Capped rather than `sm:`-gated: this sits in a pane whose width the
              viewport does not describe (§0), so a breakpoint would hide it on a
              narrow pane in a wide window and keep it on a wide pane in a narrow one. */}
          {remote && (
            <span className="max-w-[12rem] truncate font-mono text-[10px] text-slate-500">
              {remote.machineName} · run {remote.run.run_id}
            </span>
          )}
          <span
            data-testid="activity-sync"
            title={sync.hint}
            className={`flex items-center gap-1.5 font-mono text-[10px] ${TONE_TEXT[sync.tone]}`}
          >
            <span
              aria-hidden="true"
              className={`w-1.5 h-1.5 rounded-full shrink-0 bg-current ${
                sync.live ? 'animate-pulse motion-reduce:animate-none' : ''
              }`}
            />
            {sync.label}
          </span>
        </>
      }
      bodyClassName="max-h-64 overflow-y-auto px-4 pb-4 pt-3 font-mono text-xs space-y-2"
    >
      {error && machineName && (
        <p className="text-ruby-300 break-all">
          {terminal
            ? `Couldn't fetch the log from ${machineName}: ${error}.`
            : `Lost the connection to ${machineName}: ${error}. Still retrying — events shown so far are not lost.`}
        </p>
      )}
      {!error && (
        <RunEventFeed
          events={events}
          emptyHint={
            remote
              ? 'Waiting for events…'
              : 'No activity has been recorded for this run.'
          }
        />
      )}
      <div ref={bottomRef} />
    </Disclosure>
  );
}

export default ActivityPanel;
