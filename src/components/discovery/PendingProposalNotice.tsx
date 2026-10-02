import React from 'react';

import { pendingProposalNote } from '../../lib/decomposeReview';
import type { DecomposeProposal } from '../../types';

interface PendingProposalNoticeProps {
  proposal: DecomposeProposal;
  onReview: () => void;
  onDiscard: () => void;
  /** Ask for a fresh pass. Offered only over a stopped one, which has nothing
   *  to review. */
  onDecompose: () => void;
  busy: boolean;
}

/**
 * A decompose pass that finished while nobody was looking at it.
 *
 * The pass is stored against the Discovery, so this is what the workspace
 * opens with when one is outstanding — a press the user paid for and left, and
 * which used to be dropped on the floor with the component that awaited it.
 *
 * **Reviewing and discarding are two different presses.** Closing the review
 * keeps the proposal, because a pass costs minutes and dollars and the reason
 * to leave it is usually to go and look at something; discarding is the only
 * thing that forgets it, and it has to exist or a proposal reappears every
 * time this view mounts.
 *
 * **A stopped pass is not reviewable.** It carries a reason and a spend but no
 * plan, so Review is replaced by Decompose again — opening the modal over it
 * would offer a review of nothing.
 */
export function PendingProposalNotice({
  proposal,
  onReview,
  onDiscard,
  onDecompose,
  busy,
}: PendingProposalNoticeProps): React.ReactElement {
  const stopped = Boolean(proposal.stopped);
  return (
    <div
      data-testid="pending-proposal"
      role={stopped ? 'status' : undefined}
      className={`flex shrink-0 items-center justify-between gap-5 border-b px-6 py-2.5 ${
        stopped ? 'border-ruby-500/20 bg-ruby-500/5' : 'border-cyan-500/20 bg-cyan-500/5'
      }`}
    >
      <p
        className={`m-0 min-w-0 text-[11px] leading-relaxed ${stopped ? 'text-ruby-200' : 'text-slate-300'}`}
      >
        {pendingProposalNote(proposal)}
      </p>
      <div className="flex shrink-0 items-center gap-2.5">
        <button
          type="button"
          data-testid="pending-proposal-discard"
          onClick={onDiscard}
          disabled={busy}
          className="btn-secondary text-[13px] disabled:cursor-not-allowed disabled:opacity-35"
        >
          {stopped ? 'Dismiss' : 'Discard'}
        </button>
        {stopped ? (
          <button
            type="button"
            data-testid="pending-proposal-decompose"
            onClick={onDecompose}
            disabled={busy}
            className="btn-secondary text-[13px] disabled:cursor-not-allowed disabled:opacity-35"
          >
            Decompose again
          </button>
        ) : (
          <button
            type="button"
            data-testid="pending-proposal-review"
            onClick={onReview}
            disabled={busy}
            className="btn-secondary text-[13px] disabled:cursor-not-allowed disabled:opacity-35"
          >
            Review
          </button>
        )}
      </div>
    </div>
  );
}

export default PendingProposalNotice;
