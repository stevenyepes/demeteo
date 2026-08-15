import { useCallback, useEffect, useState, type ReactElement } from 'react';
import { Wrench } from 'lucide-react';

import { planFixLaunch, type FixLaunchPlan } from '../../lib/fixLaunch';
import { artifactBody } from '../../lib/features';
import { getFeature } from '../../lib/featureSync';
import { listOpenPullRequests, type PullRequestSummary } from '../../lib/pullRequests';
import { getProposedStrategy } from '../../lib/project';
import { formatError } from '../../lib/errors';
import { PostReviewComment } from './PostReviewComment';
import type { StepExecution } from '../../types';

/**
 * Turn a finished review into a run that acts on it.
 *
 * ## How this surface knows which pull request was reviewed
 *
 * Nothing persists the link. `FeatureOrigin::Ref` carries `fetch_spec` and a
 * `label`, and the label is documented as decoration — nothing derives from it.
 * So the join is the fetch spec: Demeteo's own provider mapping produced
 * `MrSummary::head_fetch_spec`, `reviewLaunch.ts` copied that value into the
 * origin at launch, and both sides are Demeteo's own. Matching on it recovers
 * the whole `PullRequestSummary` — `from_fork`, `head_repo_push`,
 * `target_branch` — which is what `planFixLaunch` needs and what no column
 * holds.
 *
 * The alternative was parsing the pull request back out of `feature.description`,
 * and that is unsafe rather than merely ugly: the request's own title is quoted
 * into that same text, so a title reading `URL: https://elsewhere/...` is
 * indistinguishable from the real line. The fetch spec is never attacker-typed.
 *
 * What this join cannot do is find a *closed* request — the listing is open
 * requests only. A review of a request that has since been merged renders
 * nothing here rather than guessing, which is the same silence as "this run was
 * not a review".
 */
export interface AddressFindingsLaunchProps {
  featureId: string;
  projectId: string | null;
  /** The finished run's steps, for the report this action seeds the fix with. */
  steps: StepExecution[];
  /** Resolves once the launch has been attempted, however it went. */
  onLaunch: (params: {
    workflowId: string;
    title: string;
    description: string;
    origin: import('../../types').FeatureOrigin;
    diffBaseBranch: string;
  }) => Promise<void>;
}

type Ready = { pullRequest: PullRequestSummary; plan: FixLaunchPlan; report: string };

const CONFIRM_TITLE = 'Start a run that addresses these findings?';

export function AddressFindingsLaunch({
  featureId,
  projectId,
  steps,
  onLaunch,
}: AddressFindingsLaunchProps): ReactElement | null {
  const [ready, setReady] = useState<Ready | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const reportPath = reviewReportPath(steps);

  useEffect(() => {
    if (!projectId || reportPath === null) {
      setReady(null);
      return;
    }
    let alive = true;

    void (async () => {
      try {
        const feature = await getFeature(featureId);
        const origin = feature?.origin;
        if (!origin || origin.kind !== 'ref') return;

        const [pullRequests, settings, findings] = await Promise.all([
          listOpenPullRequests(projectId),
          getProposedStrategy(projectId),
          artifactBody('local', reportPath),
        ]);
        const pullRequest = pullRequests.find(
          (pr) => pr.head_fetch_spec === origin.fetch_spec,
        );
        if (!alive || !pullRequest) return;

        setReady({
          pullRequest,
          report: findings,
          plan: planFixLaunch({
            pullRequest,
            findings,
            defaultBranch: settings?.worktree_strategy.default_branch ?? '',
          }),
        });
      } catch {
        // Every read here is a read this surface can do without: the action
        // simply does not appear. Surfacing an error for it would put a red
        // banner on a finished run that has nothing wrong with it.
        if (alive) setReady(null);
      }
    })();

    return () => {
      alive = false;
    };
  }, [featureId, projectId, reportPath]);

  const launch = useCallback(() => {
    if (!ready?.plan.ok || launching) return;
    setLaunching(true);
    setFailure(null);
    onLaunch(ready.plan.launch)
      .catch((err: unknown) => setFailure(formatError(err)))
      .finally(() => {
        setLaunching(false);
        setConfirming(false);
      });
  }, [ready, launching, onLaunch]);

  if (ready === null) return null;
  const { pullRequest, plan, report } = ready;

  return (
    <div className="mx-6 mt-4 rounded-xl border border-white/5 bg-black/20 px-4 py-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0 flex-1">
          <p className="font-heading text-sm font-medium text-white">
            Address the findings of this review
          </p>
          <p
            data-testid={plan.ok ? 'fix-hint' : 'fix-refused'}
            className={`mt-1 text-[11px] leading-relaxed ${
              plan.ok ? 'text-slate-500' : 'text-ruby-400'
            }`}
          >
            {plan.ok
              ? `A new run works through this report on PR #${pullRequest.number} and opens its ` +
                `own pull request against ${plan.launch.diffBaseBranch}. Every gate this project ` +
                'configures still applies.'
              : plan.message}
          </p>
        </div>

        <button
          type="button"
          data-testid="address-findings"
          disabled={!plan.ok || launching}
          onClick={() => setConfirming(true)}
          className="inline-flex shrink-0 items-center gap-2 rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white transition-all hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-40"
        >
          <Wrench aria-hidden="true" className="h-4 w-4" />
          {launching ? 'Starting…' : 'Address these findings'}
        </button>
      </div>

      {failure !== null && (
        <p data-testid="fix-failed" className="mt-2 text-[11px] text-ruby-400">
          {failure}
        </p>
      )}

      {/* The other thing a human does with a finished review, and it rides this
          surface because the pull request it needs was resolved by the fetch-spec
          join above — the one place in the app that recovers it. */}
      <div className="mt-3 border-t border-white/5 pt-3">
        <PostReviewComment
          projectId={projectId ?? ''}
          pullRequestUrl={pullRequest.web_url}
          pullRequestLabel={`PR #${pullRequest.number}`}
          report={report}
        />
      </div>

      {confirming && plan.ok && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
          <div
            role="dialog"
            aria-modal="true"
            aria-label={CONFIRM_TITLE}
            className="w-full max-w-lg rounded-xl border border-white/10 bg-slate-900/95 p-5 backdrop-blur-xl"
          >
            <h2 className="font-heading text-base font-semibold text-white">{CONFIRM_TITLE}</h2>
            <p className="mt-2 text-sm leading-relaxed text-slate-400">
              The run starts from the branch reviewed on PR #{pullRequest.number} and opens a
              pull request of its own against{' '}
              <span className="font-mono text-slate-300">{plan.launch.diffBaseBranch}</span>.
              Nothing is pushed to anyone else's branch, and every gate this project configures
              still applies.
            </p>
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setConfirming(false)}
                className="rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-sm text-slate-300 transition-colors hover:bg-white/10"
              >
                Cancel
              </button>
              <button
                type="button"
                data-testid="address-findings-confirm"
                disabled={launching}
                onClick={launch}
                className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white transition-all hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-40"
              >
                Start the run
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * The review report this run wrote, or `null` when it wrote none.
 *
 * Matched on the shipped starter's declared artifact path rather than on the
 * workflow id: the id says which starter a run *began* as, and a user who
 * copied that workflow and renamed it still produced a review report. A run
 * that wrote no such file is not a review, whatever it was called.
 */
function reviewReportPath(steps: StepExecution[]): string | null {
  for (const step of steps) {
    for (const path of step.artifact_paths) {
      if (path.endsWith('/code-review.md') || path === 'code-review.md') return path;
    }
  }
  return null;
}

export default AddressFindingsLaunch;
