import React, { useMemo, useState } from 'react';
import { Check, Lock, X } from 'lucide-react';

import {
  applyLabel,
  groupChanges,
  initialAccepted,
  lockedCount,
  passEyebrow,
  renumberNote,
  refusedChangeIds,
  toggleAccepted,
  validationState,
  violationFor,
} from '../../lib/decomposeReview';
import { applyDecomposition } from '../../lib/discovery';
import { ticketLabel } from '../../lib/discoveryProgress';
import { formatError } from '../../lib/errors';
import { TONE_TEXT } from '../../lib/runStatus';
import { stateLabel, ticketTone, type TicketIndex } from '../../lib/ticketPresentation';
import type { DecomposeProposal, DiscoveryBoard } from '../../types';
import { Chip } from '../ui/Chip';
import { Modal } from '../ui/Modal';
import { DecomposeChangeCard } from './DecomposeChangeCard';

interface DecomposeModalProps {
  proposal: DecomposeProposal;
  /** The board as it stands, for the lane a locked ticket wears. A ticket can
   *  start while this modal is open, which is why the chip is read from the
   *  live board rather than from the proposal it was rendered with. */
  index: TicketIndex;
  onClose: () => void;
  onApplied: (board: DiscoveryBoard) => void;
}

/**
 * The proposed-changes review (`DISCOVERY_UI_SPEC.md` §4): a diff to accept in
 * parts, not a wizard.
 *
 * **Nothing here caches, polls or checks the proposal for staleness.**
 * Applying hands `tickets` straight back and the backend re-resolves and
 * re-diffs it against the rows as they stand at that moment — so a ticket
 * started while this was open is refused server-side rather than silently
 * rewritten. That is also what makes the stored proposal behind this modal
 * safe to keep between visits: it is a view awaiting review, never an answer.
 * Closing keeps it; only Discard forgets it.
 *
 * **A subset of a valid proposal is not itself valid.** Declining a new ticket
 * that another accepted one is `blocked_by` leaves an edge pointing at
 * nothing, and so does accepting a removal something still waits on. Neither
 * is a mistake the agent made, so neither is re-asked: the refusal comes back
 * naming the tickets, and it is drawn on the checkboxes that caused it.
 */
export function DecomposeModal({
  proposal,
  index,
  onClose,
  onApplied,
}: DecomposeModalProps): React.ReactElement {
  const [accepted, setAccepted] = useState(() => initialAccepted(proposal.changes));
  const [applying, setApplying] = useState(false);
  const [refusal, setRefusal] = useState<string | null>(null);

  const groups = useMemo(() => groupChanges(proposal.changes), [proposal.changes]);
  const validation = useMemo(() => validationState(proposal), [proposal]);
  const implicated = useMemo(
    () => (refusal === null ? new Set<string>() : refusedChangeIds(refusal, proposal.changes)),
    [refusal, proposal.changes],
  );

  const seqOf = useMemo(() => {
    const bySeq = new Map(
      proposal.changes.filter((c) => c.seq !== null).map((c) => [c.id, c.seq as number]),
    );
    return (id: string) => bySeq.get(id) ?? null;
  }, [proposal.changes]);

  async function apply() {
    setApplying(true);
    setRefusal(null);
    try {
      const board = await applyDecomposition({
        discovery_id: proposal.discovery_id,
        tickets: proposal.tickets,
        accept: [...accepted],
      });
      onApplied(board);
    } catch (cause) {
      setRefusal(formatError(cause));
    } finally {
      setApplying(false);
    }
  }

  const applicable = !validation.fatal && accepted.size > 0 && !applying;

  return (
    <Modal onClose={onClose} className="w-full max-w-[1040px] px-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Proposed changes"
        className="glass-panel flex max-h-[88vh] flex-col overflow-hidden"
      >
        <header className="flex shrink-0 items-start justify-between gap-5 border-b border-white/5 px-6 py-5">
          <div className="min-w-0">
            <p className="m-0 mb-1.5 font-heading text-[11px] font-semibold uppercase tracking-[0.15em] text-cyan-400">
              {passEyebrow(proposal.first_pass)}
            </p>
            <h2 className="m-0 font-heading text-[22px] font-bold text-white">Proposed changes</h2>
            <p className="m-0 mt-2 max-w-[620px] text-xs leading-relaxed text-slate-400">
              Nothing here is applied until you apply it. Tickets that already have a feature cannot
              be revised, removed or renumbered — they are listed so you can see what the
              interviewer worked around.
            </p>
          </div>
          <button
            type="button"
            aria-label="Close"
            onClick={onClose}
            className="btn-secondary shrink-0 px-2.5 py-1.5"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        </header>

        <div
          data-testid="decompose-validation"
          className={`flex shrink-0 items-start gap-4 border-b border-white/5 px-6 py-3 ${
            validation.fatal ? 'bg-ruby-500/[0.04]' : 'bg-emerald-500/[0.04]'
          }`}
        >
          <Chip
            size="sm"
            tone={validation.tone}
            icon={<Check className="h-2.5 w-2.5" aria-hidden="true" />}
          >
            {validation.chip}
          </Chip>
          <div className="min-w-0">
            <p className="m-0 text-[11px] leading-relaxed text-slate-400">{validation.sentence}</p>
            {validation.details.map((detail) => (
              <p key={detail} className="m-0 mt-1 font-mono text-[11px] text-slate-300">
                {detail}
              </p>
            ))}
          </div>
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-[22px] overflow-y-auto px-6 py-5">
          {groups.map((group) => (
            <section key={group.kind}>
              <div className="mb-2.5 flex items-center gap-2.5">
                <span
                  className={`font-mono text-[10px] font-bold uppercase tracking-[0.08em] ${TONE_TEXT[group.tone]}`}
                >
                  {group.label}
                </span>
                <span className="font-mono text-[10px] text-slate-600">{group.count}</span>
              </div>
              <div className="flex flex-col gap-2.5">
                {group.changes.map((change) => (
                  <DecomposeChangeCard
                    key={change.id}
                    change={change}
                    accepted={accepted.has(change.id)}
                    refused={implicated.has(change.id)}
                    disabled={applying || validation.fatal}
                    seqOf={seqOf}
                    onToggle={() => setAccepted((current) => toggleAccepted(current, change.id))}
                  />
                ))}
              </div>
            </section>
          ))}

          {proposal.locked.length > 0 && (
            <section>
              <div className="mb-2.5 flex items-center gap-2.5">
                <span className="font-mono text-[10px] font-bold uppercase tracking-[0.08em] text-slate-500">
                  Locked
                </span>
                <span className="font-mono text-[10px] text-slate-600">
                  {lockedCount(proposal.locked.length)}
                </span>
              </div>
              <div className="flex flex-col gap-2.5">
                {proposal.locked.map((locked) => {
                  const live = index.get(locked.id);
                  const violation = violationFor(locked, proposal.violations);
                  return (
                    <div
                      key={locked.id}
                      data-testid="decompose-locked"
                      className="flex items-start gap-3.5 rounded-xl border border-white/5 bg-[#050608]/60 px-4 py-3.5"
                    >
                      <span
                        aria-hidden="true"
                        className="mt-0.5 flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-[5px] border border-dashed border-white/[0.08] text-slate-600"
                      >
                        <Lock className="h-2.5 w-2.5" aria-hidden="true" />
                      </span>
                      <div className="min-w-0 flex-1">
                        <div className="flex items-baseline gap-2.5">
                          <span className="shrink-0 font-mono text-[10px] text-slate-500">
                            {ticketLabel(locked.seq)}
                          </span>
                          <span className="min-w-0 flex-1 text-sm text-slate-400">
                            {locked.title}
                          </span>
                        </div>
                        {violation && (
                          <p className="m-0 mt-1.5 text-[11px] leading-relaxed text-ruby-200">
                            {violation.reason}
                          </p>
                        )}
                      </div>
                      {live && (
                        <Chip size="sm" tone={ticketTone(live, index)}>
                          {stateLabel(live)}
                        </Chip>
                      )}
                    </div>
                  );
                })}
              </div>
            </section>
          )}
        </div>

        {refusal && (
          <p
            role="alert"
            data-testid="decompose-refusal"
            className="m-0 shrink-0 border-t border-ruby-500/20 bg-ruby-500/5 px-6 py-2.5 text-[11px] leading-relaxed text-ruby-200"
          >
            {refusal}
          </p>
        )}

        <footer className="flex shrink-0 items-center justify-between gap-5 border-t border-white/5 bg-[#0d0f14]/90 px-6 py-4">
          <p className="m-0 text-[11px] text-slate-500">
            {renumberNote(proposal)}
          </p>
          <div className="flex shrink-0 items-center gap-2.5">
            <button type="button" onClick={onClose} className="btn-secondary text-[13px]">
              Keep talking
            </button>
            <button
              type="button"
              data-testid="decompose-apply"
              disabled={!applicable}
              onClick={() => void apply()}
              className="btn-primary text-[13px] disabled:cursor-not-allowed disabled:opacity-35"
            >
              {applying ? 'Applying…' : applyLabel(accepted.size, proposal.changes.length)}
            </button>
          </div>
        </footer>
      </div>
    </Modal>
  );
}

export default DecomposeModal;
