import { useCallback, useId, useState, type ReactElement } from 'react';

import { FieldLabel } from '../ui/FieldLabel';
import { HarnessPersonalizationNote } from './HarnessPersonalizationNote';
import { useAgentCatalog } from '../../lib/agentCatalog';
import {
  planReviewLaunch,
  REVIEW_STARTER_KEEPS_PERSONALIZATION,
  type ReviewLaunchParams,
} from '../../lib/reviewLaunch';
import type { PullRequestSummary } from '../../lib/pullRequests';

/**
 * What Demeteo is and is not responsible for in a review, said beside the
 * button that starts one rather than in a doc nobody opens first.
 *
 * Not an alert, not amber, no `role="alert"`: it reports no problem, and a
 * warning colour spent on a standing fact is a colour the user learns to
 * ignore. It holds only what is true of every harness; what the chosen one
 * brings, and what Demeteo's flags do to it, is `HarnessPersonalizationNote`
 * beneath it.
 */
export const REVIEW_SOURCE_HINT =
  'Demeteo hands the agent the diff range and a path for the report, and encodes no ' +
  "review criteria of its own. What it looks for comes from your repo's conventions " +
  'file — AGENTS.md / CLAUDE.md, which every harness reads — and from the harness itself.';

const INSTRUCTIONS_PLACEHOLDER =
  'Focus the review — e.g. "concentrate on the auth changes". Leave blank for a full review.';

export interface PullRequestLaunchProps {
  pullRequest: PullRequestSummary;
  /** Resolves once the launch has been attempted, however it went: the row
   *  stays busy until then, and a failed launch leaves it standing where it
   *  was rather than clearing what the user typed. */
  onReview: (params: ReviewLaunchParams) => Promise<void>;
  /** The harness this review will actually run on — the project's stored
   *  default, since nothing on this surface chooses one. Empty when the project
   *  has stored none: the run then falls to a built-in fallback this frontend
   *  would have to hard-code to name, so it says nothing instead. */
  agentKind: string;
}

/**
 * The launch control for one pull request, and the optional instructions that
 * ride along with it.
 *
 * The instructions field is closed by default and the primary button launches
 * from either state, so the common case — review this, as it stands — is one
 * click and the field is never in the way of it.
 *
 * There is deliberately no harness picker here. Offering one means answering
 * "which harnesses may be offered", and the only honest source for that is the
 * machine's probed, enabled agents — which this surface has no machine to ask
 * (see `StartFeatureModal`'s `agentOptions`, which does). A picker fed from the
 * bare catalog would list harnesses the user disabled and harnesses that are
 * not installed, and the note beneath it would then describe, confidently, a
 * run that dies at spawn. So the review runs on the project's default and the
 * note states what that costs.
 */
export function PullRequestLaunch({
  pullRequest,
  onReview,
  agentKind,
}: PullRequestLaunchProps): ReactElement {
  const [open, setOpen] = useState(false);
  const [instructions, setInstructions] = useState('');
  const [launching, setLaunching] = useState(false);
  const fieldId = useId();
  const { agents } = useAgentCatalog();

  const plan = planReviewLaunch(pullRequest, instructions);

  const review = useCallback(() => {
    if (!plan.ok || launching) return;
    setLaunching(true);
    void onReview(plan.launch).finally(() => setLaunching(false));
  }, [plan, launching, onReview]);

  return (
    <div className="space-y-3 border-t border-white/5 px-4 py-3">
      {open && (
        <div>
          <FieldLabel htmlFor={fieldId}>Extra instructions (optional)</FieldLabel>
          <textarea
            id={fieldId}
            rows={2}
            value={instructions}
            onChange={(e) => setInstructions(e.target.value)}
            placeholder={INSTRUCTIONS_PLACEHOLDER}
            className="w-full resize-y rounded-lg border border-white/10 bg-black/40 px-3 py-2 font-mono text-sm text-slate-200 placeholder:text-slate-600 focus:border-cyan-500/50 focus:outline-none"
          />
        </div>
      )}

      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0 flex-1 space-y-2">
          <p
            data-testid={plan.ok ? 'review-hint' : 'review-refused'}
            className={`text-[11px] leading-relaxed ${plan.ok ? 'text-slate-500' : 'text-ruby-400'}`}
          >
            {plan.ok ? REVIEW_SOURCE_HINT : plan.message}
          </p>
          <HarnessPersonalizationNote
            agents={agents}
            kind={agentKind}
            stepKeepsPersonalization={REVIEW_STARTER_KEEPS_PERSONALIZATION}
          />
        </div>

        <div className="flex shrink-0 items-end gap-2">
          <button
            type="button"
            aria-expanded={open}
            onClick={() => setOpen(!open)}
            className="rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-sm text-slate-300 transition-colors hover:bg-white/10"
          >
            {open ? 'Hide instructions' : 'Add instructions'}
          </button>
          <button
            type="button"
            data-testid="review-this-pr"
            disabled={!plan.ok || launching}
            onClick={review}
            className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white transition-all hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {launching ? 'Starting review…' : 'Review this PR'}
          </button>
        </div>
      </div>
    </div>
  );
}

export default PullRequestLaunch;
