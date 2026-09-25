import { useCallback, useId, useMemo, useState, type ReactElement } from 'react';

import { FieldLabel } from '../ui/FieldLabel';
import { HarnessPersonalizationNote } from './HarnessPersonalizationNote';
import { ReviewRunOptions, type ReviewRunInputs } from './ReviewRunOptions';
import { useRunChoice } from '../../hooks/useRunChoice';
import { useAgentCatalog } from '../../lib/agentCatalog';
import type { AgentAvailability } from '../../lib/featureDetail';
import {
  planReviewLaunch,
  REVIEW_STARTER_KEEPS_PERSONALIZATION,
  reviewWorkflowChoices,
  runChoiceGap,
  type ReviewLaunchParams,
} from '../../lib/reviewLaunch';
import type { PullRequestSummary } from '../../lib/pullRequests';
import type { WorkflowWithSteps } from '../../types';

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
  'Demeteo hands the agent the diff range and a path for the report, runs this ' +
  "project's own preparation and gate commands on the branch, and encodes no review " +
  "criteria of its own. What it looks for comes from your repo's conventions file — " +
  'AGENTS.md / CLAUDE.md, which every harness reads — and from the harness itself.';

/**
 * What one click on a fork's pull request executes, and where. The review's
 * gate step runs `prepare_command` and the gate commands in a worktree of the
 * PR's head, under the user's login shell and outside `PermissionPolicyPort` —
 * so a `postinstall` or a test file the fork's author wrote runs before anyone
 * has read the diff. Whether that should happen at all is open
 * (docs/OPEN_QUESTIONS.md); until it is decided, the person clicking decides,
 * per launch. The acknowledgement is cleared once its launch resolves, because
 * no head sha reaches this surface to bind it to, and a fork can push between
 * two launches. Consent, not an error, so it is not coloured as one.
 */
export const FORK_EXPOSURE =
  "this project's preparation and gate commands against the fork's code, on the machine " +
  'this project runs on.';

export const REVIEW_FORK_NOTICE = `This pull request comes from a fork. Reviewing it runs ${FORK_EXPOSURE}`;

const FORK_CONSENT_LABEL = 'I understand, run them against this code';

const INSTRUCTIONS_PLACEHOLDER =
  'Focus the review — e.g. "concentrate on the auth changes". Leave blank for a full review.';

/** Stable across renders so `useRunChoice`'s identity contract holds for a
 *  parent that fetched nothing; a fresh `[]` per render would not break it
 *  today, and depending on that is how it stops holding. */
const NO_MACHINE_AGENTS: AgentAvailability[] = [];
const NO_WORKFLOWS: WorkflowWithSteps[] = [];

export interface PullRequestLaunchProps {
  pullRequest: PullRequestSummary;
  /** Resolves once the launch has been attempted, however it went: the row
   *  stays busy until then, and a failed launch leaves it standing where it
   *  was rather than clearing what the user typed. */
  onReview: (params: ReviewLaunchParams) => Promise<void>;
  /** The project's stored default harness — what this review runs on while the
   *  harness control is left inherited, which is the common case even now that
   *  the control exists. Empty when the project has stored none: the run then
   *  falls to a built-in fallback this frontend would have to hard-code to
   *  name, so it says nothing instead. */
  agentKind: string;
  /** What the view fetched once for every row in it. Absent means the parent
   *  has not fetched it, or the read failed — no controls render and the run
   *  shape is the project's throughout, which is the whole behaviour this
   *  surface had before there was a picker. */
  runOptions?: ReviewRunInputs;
}

/**
 * The launch control for one pull request, and the optional instructions that
 * ride along with it.
 *
 * The panel is closed by default and the primary button launches from either
 * state, so the common case — review this, as it stands, on the project's own
 * defaults — is one click and nothing in it is ever in the way of that.
 *
 * For most of this surface's life there was no harness picker, and the argument
 * against one was not that it is noise. It was that offering one means
 * answering "which harnesses may be offered", whose only honest source is the
 * machine's probed, enabled agents — a machine this surface had no way to ask
 * (see `StartFeatureModal`'s `agentOptions`, which could). Fed from the bare
 * catalog instead, a picker lists harnesses the user disabled and harnesses
 * that are not installed, and the note beneath it then describes, confidently,
 * a run that dies at spawn.
 *
 * What changed is the source, not that judgement. `runOptions` carries this
 * view's one `listAgentConfigs` answer for its machine, which `useRunChoice`
 * narrows to `enabled && available` — exactly the machine the objection said
 * was out of reach. The objection still decides the absent case: with no
 * `runOptions` no control renders at all, rather than a catalog-fed guess.
 */
export function PullRequestLaunch({
  pullRequest,
  onReview,
  agentKind,
  runOptions,
}: PullRequestLaunchProps): ReactElement {
  const [open, setOpen] = useState(false);
  const [instructions, setInstructions] = useState('');
  const [launching, setLaunching] = useState(false);
  // Keyed on the pull request rather than a bare boolean, so a row handed a
  // different one reads as unacknowledged on that very render, without an
  // effect racing the click.
  const [acknowledgedFor, setAcknowledgedFor] = useState<string | null>(null);
  const fieldId = useId();
  const { agents } = useAgentCatalog();
  const runChoice = useRunChoice({
    machineAgents: runOptions?.machineAgents ?? NO_MACHINE_AGENTS,
    machineId: runOptions?.machineId ?? '',
  });
  const workflowChoices = useMemo(
    () => reviewWorkflowChoices(runOptions?.workflows ?? NO_WORKFLOWS),
    [runOptions?.workflows],
  );

  const plan = planReviewLaunch(pullRequest, instructions, runChoice.choice);
  const modelGap = runChoiceGap(runChoice.choice);
  const awaitingConsent = pullRequest.from_fork && acknowledgedFor !== pullRequest.web_url;

  const review = useCallback(() => {
    if (!plan.ok || modelGap !== null || launching || awaitingConsent) return;
    setLaunching(true);
    void onReview(plan.launch).finally(() => {
      setLaunching(false);
      setAcknowledgedFor(null);
    });
  }, [plan, modelGap, launching, awaitingConsent, onReview]);

  return (
    <div className="space-y-3 border-t border-white/5 px-4 py-3">
      {open && (
        <div className="space-y-3">
          {runOptions && <ReviewRunOptions choice={runChoice} workflows={workflowChoices} />}
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
        </div>
      )}

      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0 flex-1 space-y-2">
          {!plan.ok ? (
            <p data-testid="review-refused" className="text-[11px] leading-relaxed text-ruby-400">
              {plan.message}
            </p>
          ) : modelGap !== null ? (
            <p
              data-testid="review-model-required"
              className="text-[11px] leading-relaxed text-slate-400"
            >
              {modelGap}
            </p>
          ) : (
            <p data-testid="review-hint" className="text-[11px] leading-relaxed text-slate-500">
              {REVIEW_SOURCE_HINT}
            </p>
          )}
          {pullRequest.from_fork && (
            <div className="space-y-1.5 rounded-lg border border-violet-500/30 bg-violet-500/10 px-3 py-2">
              <p
                data-testid="review-fork-notice"
                className="text-[11px] leading-relaxed text-violet-200"
              >
                {REVIEW_FORK_NOTICE}
              </p>
              <label className="flex cursor-pointer items-center gap-2 text-xs text-slate-300">
                <input
                  type="checkbox"
                  data-testid="review-fork-consent"
                  checked={!awaitingConsent}
                  onChange={(e) =>
                    setAcknowledgedFor(e.target.checked ? pullRequest.web_url : null)
                  }
                  className="accent-violet-500"
                />
                {FORK_CONSENT_LABEL}
              </label>
            </div>
          )}
          <HarnessPersonalizationNote
            agents={agents}
            kind={runChoice.agentKind || agentKind}
            stepKeepsPersonalization={REVIEW_STARTER_KEEPS_PERSONALIZATION}
          />
        </div>

        <div className="flex shrink-0 items-end gap-2">
          <button
            type="button"
            data-testid="review-options-toggle"
            aria-expanded={open}
            onClick={() => setOpen(!open)}
            className="rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-sm text-slate-300 transition-colors hover:bg-white/10"
          >
            {togglePanelLabel(open, runOptions !== undefined)}
          </button>
          <button
            type="button"
            data-testid="review-this-pr"
            disabled={!plan.ok || modelGap !== null || launching || awaitingConsent}
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

/** The panel holds only the instructions field when the parent fetched no run
 *  options, and a button promising options that are not behind it is a control
 *  the user hunts for and does not find. */
function togglePanelLabel(open: boolean, hasRunOptions: boolean): string {
  if (hasRunOptions) return open ? 'Hide options' : 'Options';
  return open ? 'Hide instructions' : 'Add instructions';
}

export default PullRequestLaunch;
