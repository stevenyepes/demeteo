import { useCallback, useEffect, useMemo, useState, type ReactElement } from 'react';
import { Wrench } from 'lucide-react';

import {
  fixWorkflowChoices,
  planFixLaunch,
  type FixLaunchParams,
  type FixLaunchPlan,
} from '../../lib/fixLaunch';
import { artifactBody } from '../../lib/features';
import { getFeature } from '../../lib/featureSync';
import { getProjectById, listAgentConfigs } from '../../lib/featureDetail';
import { listOpenPullRequests, type PullRequestSummary } from '../../lib/pullRequests';
import { getProposedStrategy } from '../../lib/project';
import {
  reviewArtifactPaths,
  seedFindings,
  type GateEvidence,
  type ReviewEvidence,
} from '../../lib/reviewEvidence';
import { runChoiceGap } from '../../lib/reviewLaunch';
import { listWorkflows } from '../../lib/workflows';
import { formatError } from '../../lib/errors';
import { PostReviewComment } from './PostReviewComment';
import { FORK_EXPOSURE } from './PullRequestLaunch';
import { ReviewRunOptions, type ReviewRunInputs } from './ReviewRunOptions';
import { useRunChoice } from '../../hooks/useRunChoice';
import type { StepExecution, WorkflowWithSteps } from '../../types';

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
  /** The finished run's steps, for the artifacts this action seeds the fix with. */
  steps: StepExecution[];
  /** The run's own status, which decides whether an unfinished gate step is
   *  still worth waiting for — see `reviewArtifactPaths`. */
  runStatus: string;
  /** Resolves once the launch has been attempted, however it went. */
  onLaunch: (params: FixLaunchParams) => Promise<void>;
}

/** What the join recovered. The plan is *not* held here: it reads the run-shape
 *  pickers, which change long after this resolved. `builtFor` is the
 *  `joinKey` it was read under: a ready from before the gate step settled has
 *  no gates in it, and must hold the launch until the re-join replaces it. */
type Ready = {
  pullRequest: PullRequestSummary;
  evidence: ReviewEvidence;
  defaultBranch: string;
  builtFor: string;
};

const CONFIRM_TITLE = 'Start a run that addresses these findings?';

/** One object for the un-probed and the failed-probe case alike, so
 *  `useRunChoice`'s identity contract does not rest on a fresh `[]` per
 *  render. */
const NO_RUN_INPUTS: ReviewRunInputs = { workflows: [], machineAgents: [], machineId: '' };

export function AddressFindingsLaunch({
  featureId,
  projectId,
  steps,
  runStatus,
  onLaunch,
}: AddressFindingsLaunchProps): ReactElement | null {
  const [ready, setReady] = useState<Ready | null>(null);
  const [runInputs, setRunInputs] = useState<ReviewRunInputs>(NO_RUN_INPUTS);
  const [confirming, setConfirming] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const {
    report: reportPath,
    gates: gatesPath,
    gateFailure,
    gatePending,
  } = reviewArtifactPaths(steps, runStatus);
  const key = joinKey(reportPath, gatesPath, gateFailure, gatePending);

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

        // The gate read is caught on its own: losing it costs the fix run its
        // gate section, not the user the action. It does not fall back to
        // `gateFailure` either — a declared artifact means the step completed,
        // so there is no red gate for a failure reason to describe. Nor is it
        // read while the step is still running: the launch is held then, and
        // `gatePending` settling re-runs this join for the final word.
        const readGates = gatesPath !== null && !gatePending;
        const [pullRequests, settings, report, gateReport] = await Promise.all([
          listOpenPullRequests(projectId),
          getProposedStrategy(projectId),
          artifactBody('local', reportPath),
          readGates ? artifactBody('local', gatesPath).catch(() => null) : null,
        ]);
        const pullRequest = pullRequests.find(
          (pr) => pr.head_fetch_spec === origin.fetch_spec,
        );
        if (!alive) return;
        if (!pullRequest) {
          setReady(null);
          return;
        }

        let gates: GateEvidence | null = null;
        if (gateReport !== null) {
          gates = { source: 'report', body: gateReport };
        } else if (!gatePending && gatesPath === null && gateFailure !== null) {
          gates = { source: 'failure', body: gateFailure };
        }

        setReady({
          pullRequest,
          evidence: { report, gates },
          defaultBranch: settings?.worktree_strategy.default_branch ?? '',
          builtFor: joinKey(reportPath, gatesPath, gateFailure, gatePending),
        });
      } catch {
        // Every read left uncaught is one this surface cannot seed a fix
        // without: the action simply does not appear. Surfacing an error for it
        // would put a red banner on a finished run that has nothing wrong with it.
        if (alive) setReady(null);
      }
    })();

    return () => {
      alive = false;
    };
  }, [featureId, projectId, reportPath, gatesPath, gateFailure, gatePending]);

  // Its own effect, and not folded into the join above: that one is wrapped so
  // that any read it needs takes the whole action away, which is the right answer
  // for reads that decide whether there is a fix to offer at all. These decide
  // only what the pickers hold, and empty pickers are a run on the project's
  // defaults — what this surface did before it had controls. Recovering the
  // machine the same way `useHarnessOverrides.probeForFeature` does, because a
  // harness list and a model list are both per-machine.
  useEffect(() => {
    if (!projectId) {
      setRunInputs(NO_RUN_INPUTS);
      return;
    }
    let alive = true;

    void (async () => {
      const project = await getProjectById(projectId).catch(() => null);
      const machineId = project?.remote_host || 'local';
      const [workflows, machineAgents] = await Promise.all([
        listWorkflows().catch(() => []),
        listAgentConfigs({ machineId, refresh: false }).catch(() => []),
      ]);
      if (alive) setRunInputs({ workflows, machineAgents, machineId });
    })();

    return () => {
      alive = false;
    };
  }, [projectId]);

  const runChoice = useRunChoice({
    machineAgents: runInputs.machineAgents,
    machineId: runInputs.machineId,
  });
  const workflowChoices = useMemo(
    () => fixWorkflowChoices(runInputs.workflows),
    [runInputs.workflows],
  );

  // Recomputed per render rather than stored beside `ready`: it reads the
  // pickers, and a plan frozen when the join resolved would carry the shape the
  // user had not chosen yet.
  const plan: FixLaunchPlan | null =
    ready === null
      ? null
      : planFixLaunch({
          pullRequest: ready.pullRequest,
          findings: seedFindings(ready.evidence),
          defaultBranch: ready.defaultBranch,
          choice: runChoice.choice,
        });

  // Kept on screen while stale rather than cleared: blanking it would take the
  // comment action away for every re-join.
  const held = gatePending || (ready !== null && ready.builtFor !== key);
  const modelGap = runChoiceGap(runChoice.choice);

  const launch = useCallback(() => {
    if (!plan?.ok || held || modelGap !== null || launching) return;
    setLaunching(true);
    setFailure(null);
    onLaunch(plan.launch)
      .catch((err: unknown) => setFailure(formatError(err)))
      .finally(() => {
        setLaunching(false);
        setConfirming(false);
      });
  }, [plan, held, modelGap, launching, onLaunch]);

  if (ready === null || plan === null) return null;
  const { pullRequest, evidence } = ready;
  const offersSomething = workflowChoices.length > 0 || runChoice.availableAgents.length > 0;

  return (
    <div className="mx-6 mt-4 rounded-xl border border-white/5 bg-black/20 px-4 py-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0 flex-1">
          <p className="font-heading text-sm font-medium text-white">
            Address the findings of this review
          </p>
          {!plan.ok ? (
            <p data-testid="fix-refused" className="mt-1 text-[11px] leading-relaxed text-ruby-400">
              {plan.message}
            </p>
          ) : held ? (
            <p
              data-testid="fix-gates-pending"
              className="mt-1 text-[11px] leading-relaxed text-slate-400"
            >
              {gatePending
                ? "This project's gates are still running on this branch; the fix can start " +
                  'once they finish.'
                : "Reading what this project's gates said about this branch…"}
            </p>
          ) : modelGap !== null ? (
            <p
              data-testid="fix-model-required"
              className="mt-1 text-[11px] leading-relaxed text-slate-400"
            >
              {modelGap}
            </p>
          ) : (
            <p data-testid="fix-hint" className="mt-1 text-[11px] leading-relaxed text-slate-500">
              {`A new run works through this report on PR #${pullRequest.number} and opens its ` +
                `own pull request against ${plan.publishesTo}. Every gate this project ` +
                'configures still applies.'}
            </p>
          )}
        </div>

        <button
          type="button"
          data-testid="address-findings"
          disabled={!plan.ok || held || modelGap !== null || launching}
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

      {plan.ok && offersSomething && (
        <div className="mt-3 border-t border-white/5 pt-3">
          <ReviewRunOptions choice={runChoice} workflows={workflowChoices} />
        </div>
      )}

      {/* The other thing a human does with a finished review, and it rides this
          surface because the pull request it needs was resolved by the fetch-spec
          join above — the one place in the app that recovers it. */}
      <div className="mt-3 border-t border-white/5 pt-3">
        <PostReviewComment
          projectId={projectId ?? ''}
          pullRequestUrl={pullRequest.web_url}
          pullRequestLabel={`PR #${pullRequest.number}`}
          report={evidence.report}
        />
      </div>

      {confirming && plan.ok && !held && modelGap === null && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
          <div
            role="dialog"
            aria-modal="true"
            aria-label={CONFIRM_TITLE}
            className="w-full max-w-lg rounded-xl border border-white/10 bg-slate-900/95 p-5 backdrop-blur-xl"
          >
            <h2 className="font-heading text-base font-semibold text-white">{CONFIRM_TITLE}</h2>
            <p className="mt-2 text-sm leading-relaxed text-slate-400">
              The{' '}
              <span data-testid="fix-chosen-workflow" className="font-mono text-slate-300">
                {workflowLabel(plan.launch.workflowId, runInputs.workflows)}
              </span>{' '}
              workflow starts from the branch reviewed on PR #{pullRequest.number} and opens a
              pull request of its own against{' '}
              <span data-testid="fix-publishes-to" className="font-mono text-slate-300">
                {plan.publishesTo}
              </span>
              .
              Nothing is pushed to anyone else's branch, and every gate this project configures
              still applies.
            </p>
            {pullRequest.from_fork && (
              <p
                data-testid="fix-fork-notice"
                className="mt-3 rounded-lg border border-violet-500/30 bg-violet-500/10 px-3 py-2 text-[11px] leading-relaxed text-violet-200"
              >
                This pull request comes from a fork, and this run starts from its code: it runs{' '}
                {FORK_EXPOSURE}
              </p>
            )}
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

function joinKey(
  reportPath: string | null,
  gatesPath: string | null,
  gateFailure: string | null,
  gatePending: boolean,
): string {
  return JSON.stringify([reportPath, gatesPath, gateFailure, gatePending]);
}

/** What to call the workflow at the moment the user confirms. Falls back to the
 *  id: the list is fetched separately and may be empty, and a dialog that names
 *  nothing is worse than one naming an id. */
function workflowLabel(workflowId: string, workflows: WorkflowWithSteps[]): string {
  return workflows.find((workflow) => workflow.id === workflowId)?.name ?? workflowId;
}

export default AddressFindingsLaunch;
