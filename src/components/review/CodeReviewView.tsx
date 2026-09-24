import { useCallback, useEffect, useRef, useState } from 'react';
import { GitPullRequest } from 'lucide-react';

import EmptyStateCard from '../EmptyStateCard';
import { PullRequestListSkeleton } from './PullRequestListSkeleton';
import { PullRequestRow } from './PullRequestRow';
import { ReviewFailureCard } from './ReviewFailureCard';
import { useNavigation, useProject } from '../../context';
import { useLaunchRun } from '../../hooks/useLaunchRun';
import { getProjectById, listAgentConfigs } from '../../lib/featureDetail';
import { getProposedStrategy } from '../../lib/project';
import { listWorkflows } from '../../lib/workflows';
import {
  asPullRequestListFailure,
  describeDetailFailure,
  listOpenPullRequests,
  loadPullRequestDetail,
  type PullRequestListFailure,
  type PullRequestSummary,
} from '../../lib/pullRequests';
import type { ReviewLaunchParams } from '../../lib/reviewLaunch';
import type { ReviewRunInputs } from './ReviewRunOptions';
import { BackButton } from '../ui/BackButton';

type ListState =
  | { status: 'loading' }
  | { status: 'ready'; pullRequests: PullRequestSummary[] }
  | { status: 'failed'; failure: PullRequestListFailure };

/**
 * Open pull requests for the current project — the queue a review run starts
 * from.
 *
 * Project-scoped, and it reads the project from context rather than from the
 * route: `AppView` holds no id for the same reason the rail is the only place
 * one is chosen. A copy on the view could disagree with the rail after a
 * project switch, and the disagreement would look like a listing that simply
 * has the wrong rows in it.
 */
export function CodeReviewView(): React.ReactElement {
  const { state: { currentProjectId, projects } } = useProject();
  const { navigate } = useNavigation();
  const [state, setState] = useState<ListState>({ status: 'loading' });
  const [defaultAgentKind, setDefaultAgentKind] = useState('');
  /** What every row's run-shape controls are offered from, held as one state
   *  object rather than assembled per render: the rows are `memo`ized and this
   *  view re-renders on every listing refresh and every enrichment landing, so
   *  a value rebuilt per render re-renders the whole queue. */
  const [runOptions, setRunOptions] = useState<ReviewRunInputs | undefined>(undefined);
  /** Why the tier stopped filling in, once one row's enrichment has failed. Not
   *  a `ListState`: the listing succeeded and its rows are all still true. */
  const [detailFailure, setDetailFailure] = useState<PullRequestListFailure | null>(null);
  const latestRead = useRef(0);
  /** Rows whose detail has been requested, so pointing at one twice is one
   *  request and a re-render is none. */
  const asked = useRef(new Set<string>());

  const projectName = projects.find((p) => p.id === currentProjectId)?.name ?? null;

  // A read the user superseded — by switching project, or by pressing Retry
  // while the first attempt was still in flight — must not land: its rows name
  // pull requests from another repository, or its failure re-states one the
  // retry has already answered, and nothing on screen would say which.
  const load = useCallback(() => {
    if (!currentProjectId) return;
    latestRead.current += 1;
    const read = latestRead.current;
    asked.current.clear();
    setDetailFailure(null);
    setState({ status: 'loading' });

    listOpenPullRequests(currentProjectId)
      .then((pullRequests) => {
        if (read === latestRead.current) setState({ status: 'ready', pullRequests });
      })
      .catch((err: unknown) => {
        if (read === latestRead.current) {
          setState({ status: 'failed', failure: asPullRequestListFailure(err) });
        }
      });
  }, [currentProjectId]);

  useEffect(load, [load]);

  // The harness every row's launch will inherit. Read here rather than per row
  // so the answer is one fetch and one value: rows disagreeing about which
  // harness the same project launches on is a lie no amount of correct copy
  // fixes. A failed read leaves it blank, which renders no claim at all — the
  // honest state, since the run would then fall to a built-in fallback this
  // frontend cannot name without duplicating `resolve_step_agent`.
  useEffect(() => {
    if (!currentProjectId) return;
    let alive = true;
    getProposedStrategy(currentProjectId)
      .then((settings) => {
        if (alive) setDefaultAgentKind(settings?.default_agent_kind ?? '');
      })
      .catch(() => {
        if (alive) setDefaultAgentKind('');
      });
    return () => {
      alive = false;
    };
  }, [currentProjectId]);

  // The same rate-limit reasoning as the enrichment below, in the shape a
  // picker takes: what a row may offer — the workflows, and the harnesses this
  // project's machine has both installed and switched on — is one answer for
  // the whole queue. Read per row, a hundred-row listing is a hundred
  // `get_agent_configs`, which for a remote project is a hundred SSH round
  // trips before the user has clicked anything.
  //
  // Either read failing leaves its own list empty rather than the object
  // absent: a rejected workflow registry still leaves the harnesses choosable.
  // Neither failure gets a banner — same convention as the harness read above,
  // and nothing here was asked for. Both empty is `undefined`, because there is
  // then nothing to choose and `PullRequestLaunch` renders no panel for it
  // rather than one whose every control offers only its own placeholder.
  //
  // A project switch drops the previous answer before asking for the next one,
  // rather than only refusing to let the old one land: `machineId` and the
  // harnesses are the *other* project's machine, and a launch that reached the
  // picker in the gap would pin a harness chosen for a machine this run will
  // not touch.
  useEffect(() => {
    setRunOptions(undefined);
    if (!currentProjectId) return;
    let alive = true;
    void (async () => {
      const project = await getProjectById(currentProjectId).catch(() => null);
      const machineId = project?.remote_host || 'local';
      const [workflows, machineAgents] = await Promise.all([
        listWorkflows().catch(() => []),
        listAgentConfigs({ machineId, refresh: false }).catch(() => []),
      ]);
      if (!alive) return;
      const offersSomething = workflows.length > 0 || machineAgents.length > 0;
      setRunOptions(offersSomething ? { workflows, machineAgents, machineId } : undefined);
    })();
    return () => {
      alive = false;
    };
  }, [currentProjectId]);

  // Enrichment is per row and on demand, never per refresh. GitHub carries
  // mergeability and the diffstat only on the single-request GET, so a queue of
  // a hundred rows enriched eagerly is a hundred serialized provider requests —
  // against a rate limit this app can detect and has no way to back off from.
  // A row asks once, when the user points at it; a failure leaves the row as
  // the listing left it, because the identity tier is still true.
  //
  // `asked` is therefore not released by a failure. Releasing it made the
  // pointer the retry loop: the one condition under which this fails at scale
  // is the throttled token, and a mouse sweeping down a hundred rows and back
  // would then issue two hundred requests into a quota that is already out —
  // with the rows no better off, because the second answer is the first one
  // again. One attempt per row per listing; `load` is the retry, and it is a
  // press.
  const requestDetail = useCallback(
    (url: string) => {
      if (!currentProjectId || asked.current.has(url)) return;
      asked.current.add(url);
      const read = latestRead.current;
      loadPullRequestDetail(currentProjectId, url)
        .then((detail) => {
          if (read !== latestRead.current) return;
          setState((prev) =>
            prev.status === 'ready'
              ? {
                  status: 'ready',
                  pullRequests: prev.pullRequests.map((pr) =>
                    pr.web_url === url ? detail : pr,
                  ),
                }
              : prev,
          );
        })
        .catch((err: unknown) => {
          if (read !== latestRead.current) return;
          setDetailFailure(asPullRequestListFailure(err));
        });
    },
    [currentProjectId],
  );

  const connectProvider = useCallback(() => navigate({ kind: 'providers' }), [navigate]);

  // The one launch code path (F28), reached from a second composer rather than
  // routed around: a review is a run like any other, and on success this
  // navigates to its feature detail the way every other launch does.
  const launchRun = useLaunchRun({ projectId: currentProjectId });
  const startReview = useCallback(
    async (params: ReviewLaunchParams) => {
      await launchRun(params);
    },
    [launchRun],
  );

  return (
    <div className="flex-1 min-w-0 overflow-y-auto p-8">
      <div className="max-w-5xl mx-auto w-full space-y-6">
        <div className="flex items-start gap-3">
          <BackButton className="mt-1" />
          <div>
            <h1 className="font-heading text-2xl font-bold text-white tracking-tight">Code Review</h1>
            <p className="mt-1 text-sm text-slate-400">
              Open pull requests{projectName ? ` in ${projectName}` : ''}.
            </p>
          </div>
        </div>

        {state.status === 'loading' && <PullRequestListSkeleton />}

        {state.status === 'failed' && (
          <ReviewFailureCard
            failure={state.failure}
            onConnect={connectProvider}
            onRetry={load}
          />
        )}

        {state.status === 'ready' && detailFailure && (
          <p
            role="status"
            data-testid="code-review-detail-failure"
            data-failure={detailFailure.kind}
            className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-4 py-3 text-sm text-slate-300"
          >
            {describeDetailFailure(detailFailure)}
          </p>
        )}

        {state.status === 'ready' &&
          (state.pullRequests.length === 0 ? (
            <EmptyStateCard
              variant="inline"
              icon={GitPullRequest}
              title="No open pull requests"
              description="Nothing is waiting for review right now. Open one and it will show up here."
            />
          ) : (
            <div className="space-y-3">
              {state.pullRequests.map((pullRequest) => (
                // Keyed on the URL, not the number: the listing spans every
                // repository in the project, and `head_fetch_spec` is derived
                // from the number alone — two repositories each with a #7 would
                // key identically and React would hand one row's memoized
                // instance the other's pull request.
                <PullRequestRow
                  key={pullRequest.web_url}
                  pullRequest={pullRequest}
                  onReview={startReview}
                  agentKind={defaultAgentKind}
                  runOptions={runOptions}
                  onRequestDetail={requestDetail}
                />
              ))}
            </div>
          ))}
      </div>
    </div>
  );
}

export default CodeReviewView;
