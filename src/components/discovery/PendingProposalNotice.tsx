import React from 'react';

import { pendingProposalNote } from '../../lib/decomposeReview';
import type { DecomposeProposal } from '../../types';

interface PendingProposalNoticeProps {
  proposal: DecomposeProposal;
  onReview: () => void;
  onDiscard: () => void;
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
 */
export function PendingProposalNotice({
  proposal,
  onReview,
  onDiscard,
  busy,
}: PendingProposalNoticeProps): React.ReactElement {
  return (
    <div
      data-testid="pending-proposal"
      className="flex shrink-0 items-center justify-between gap-5 border-b border-cyan-500/20 bg-cyan-500/5 px-6 py-2.5"
    >
      <p className="m-0 min-w-0 text-[11px] leading-relaxed text-slate-300">
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
          Discard
        </button>
        <button
          type="button"
          data-testid="pending-proposal-review"
          onClick={onReview}
          disabled={busy}
          className="btn-secondary text-[13px] disabled:cursor-not-allowed disabled:opacity-35"
        >
          Review
        </button>
      </div>
    </div>
  );
}

export default PendingProposalNotice;
