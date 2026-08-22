import React, { useState } from 'react';

import { ticketLabel } from '../../lib/discoveryProgress';
import { MIN_REASON } from '../../lib/ticketEditor';
import { primaryAction, showsForceStart, type TicketIndex } from '../../lib/ticketPresentation';
import type { TicketView } from '../../types';
import { FieldLabel } from '../ui/FieldLabel';

type Phase = 'idle' | 'forcing' | 'dropping';

interface TicketForceStartProps {
  view: TicketView;
  index: TicketIndex;
  busy: boolean;
  onStart: () => void;
  onForceStart: (reason: string) => void;
  onDrop: (reason: string) => void;
}

/**
 * §5.9's three phases, and the drop beside them.
 *
 * **The reason is the whole point.** §6.5 accepts that a project with no forge
 * remote drives its graph by hand, and the recorded reason is what keeps that
 * from becoming an unexplained bypass — for the user reading the ticket later
 * and for the agent, which is handed the same sentence in its own prerequisite
 * briefing (§7.2). So the confirm is disabled until there is one, and the hint
 * says which of the two states it is in.
 *
 * Dropping is a different act from force-starting and is pushed to the far
 * right for that reason: one starts the work anyway, the other decides against
 * it — and a dropped ticket keeps its reason and releases what waited on it
 * (§6.6), which a *removal* does not do (§4.7).
 */
export function TicketForceStart({
  view,
  index,
  busy,
  onStart,
  onForceStart,
  onDrop,
}: TicketForceStartProps): React.ReactElement | null {
  const [phase, setPhase] = useState<Phase>('idle');
  const [reason, setReason] = useState('');

  const forced = view.ticket.force_started_at !== null;
  const action = primaryAction(view, index);
  const blockers = view.standing.blockers.map((blocker) => {
    const prerequisite = index.get(blocker.id);
    return prerequisite ? ticketLabel(prerequisite.ticket.seq) : 'an unknown ticket';
  });
  const short = reason.trim().length < MIN_REASON;

  if (view.ticket.state === 'dropped') return null;

  if (forced) {
    return (
      <div
        data-testid="ticket-forced"
        className="rounded-xl border border-emerald-500/25 bg-emerald-500/5 px-4 py-3.5"
      >
        <div className="flex items-center gap-2">
          <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-emerald-400" />
          <span className="font-heading text-[13px] font-semibold text-emerald-400">
            Force-started
          </span>
        </div>
        <p className="m-0 mt-2 font-mono text-[11px] leading-loose text-slate-400">
          {new Date(view.ticket.force_started_at ?? 0).toLocaleString()}
          <br />
          {`“${view.ticket.force_start_reason ?? ''}”`}
        </p>
        <p className="m-0 mt-2.5 text-[11px] text-slate-500">
          Kept on the ticket, and repeated to the agent above.
        </p>
      </div>
    );
  }

  const asking = phase !== 'idle';
  const dropping = phase === 'dropping';

  return (
    <div className="border-t border-white/5 pt-4">
      {showsForceStart(view) && (
        <>
          <FieldLabel className="text-amber-400!">Start it anyway</FieldLabel>
          <p className="m-0 mb-3 text-xs leading-relaxed text-slate-400">
            {blockers.length > 0
              ? `${blockers.join(', ')} ${blockers.length === 1 ? 'has' : 'have'} not released this ticket. `
              : 'Nothing has released this ticket yet. '}
            Force start bypasses every edge at once — and records why, for you and for the agent
            that reads its own prerequisite list.
          </p>
        </>
      )}

      {asking ? (
        <div
          className={`rounded-xl border px-4 py-3.5 ${
            dropping ? 'border-white/10 bg-white/[0.02]' : 'border-amber-500/25 bg-amber-500/5'
          }`}
        >
          <FieldLabel className={dropping ? '' : 'text-amber-400!'}>
            {dropping
              ? `Why are you dropping ${ticketLabel(view.ticket.seq)}?`
              : `Why are you bypassing ${blockers.length > 0 ? blockers.join(', ') : 'its edges'}?`}
          </FieldLabel>
          <textarea
            rows={3}
            value={reason}
            onChange={(event) => setReason(event.target.value)}
            aria-label={dropping ? 'Reason for dropping this ticket' : 'Reason for bypassing the prerequisites'}
            placeholder={
              dropping
                ? 'Folded into the ticket below it — this one has nothing left to do.'
                : `This project has no forge remote — ${blockers[0] ?? 'it'} merged out of band this morning.`
            }
            className="input-field"
          />
          <div className="mt-3 flex items-center gap-2.5">
            <button
              type="button"
              data-testid={dropping ? 'ticket-drop-confirm' : 'ticket-force-confirm'}
              disabled={short || busy}
              onClick={() => (dropping ? onDrop(reason.trim()) : onForceStart(reason.trim()))}
              className={
                dropping
                  ? 'btn-secondary disabled:cursor-not-allowed disabled:opacity-35'
                  : 'rounded-md border border-amber-500/25 bg-amber-500/[0.08] px-[18px] py-2.5 text-[13px] font-medium text-amber-400 transition hover:bg-amber-500/[0.14] disabled:cursor-not-allowed disabled:opacity-35'
              }
            >
              {dropping
                ? `Drop ${ticketLabel(view.ticket.seq)}`
                : `Force start ${ticketLabel(view.ticket.seq)}`}
            </button>
            <button
              type="button"
              onClick={() => {
                setPhase('idle');
                setReason('');
              }}
              className="btn-secondary"
            >
              Cancel
            </button>
            <span className="ml-auto text-[11px] text-slate-500">
              {short ? 'A reason is required.' : 'Recorded on the ticket.'}
            </span>
          </div>
        </div>
      ) : (
        <div className="flex items-center gap-2.5">
          <button
            type="button"
            data-testid="ticket-primary-action"
            disabled={action.kind !== 'start' || busy}
            onClick={onStart}
            className="btn-primary text-[13px] disabled:cursor-not-allowed disabled:opacity-35"
          >
            {action.label}
          </button>
          {showsForceStart(view) && (
            <button
              type="button"
              data-testid="ticket-force-start"
              disabled={busy}
              onClick={() => setPhase('forcing')}
              className="rounded-md border border-amber-500/25 bg-amber-500/[0.08] px-[18px] py-2.5 text-[13px] font-medium text-amber-400 transition hover:bg-amber-500/[0.14] disabled:cursor-not-allowed disabled:opacity-35"
            >
              Force start&hellip;
            </button>
          )}
          <button
            type="button"
            data-testid="ticket-drop"
            disabled={busy}
            onClick={() => setPhase('dropping')}
            className="btn-secondary ml-auto text-[13px] disabled:cursor-not-allowed disabled:opacity-35"
          >
            Drop ticket&hellip;
          </button>
        </div>
      )}
    </div>
  );
}

export default TicketForceStart;
