import { useCallback, useEffect, useRef, useState } from 'react';
import { GitPullRequest } from 'lucide-react';

import EmptyStateCard from '../EmptyStateCard';
import { PullRequestListSkeleton } from './PullRequestListSkeleton';
import { PullRequestRow } from './PullRequestRow';
import { ReviewFailureCard } from './ReviewFailureCard';
import { useNavigation, useProject } from '../../context';
import {
  asPullRequestListFailure,
  listOpenPullRequests,
  type PullRequestListFailure,
  type PullRequestSummary,
} from '../../lib/pullRequests';

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
  const latestRead = useRef(0);

  const projectName = projects.find((p) => p.id === currentProjectId)?.name ?? null;

  // A read the user superseded — by switching project, or by pressing Retry
  // while the first attempt was still in flight — must not land: its rows name
  // pull requests from another repository, or its failure re-states one the
  // retry has already answered, and nothing on screen would say which.
  const load = useCallback(() => {
    if (!currentProjectId) return;
    latestRead.current += 1;
    const read = latestRead.current;
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

  const connectProvider = useCallback(() => navigate({ kind: 'providers' }), [navigate]);

  return (
    <div className="flex-1 min-w-0 overflow-y-auto p-8">
      <div className="max-w-5xl mx-auto w-full space-y-6">
        <div>
          <h1 className="font-heading text-2xl font-bold text-white tracking-tight">Code Review</h1>
          <p className="mt-1 text-sm text-slate-400">
            Open pull requests{projectName ? ` in ${projectName}` : ''}.
          </p>
        </div>

        {state.status === 'loading' && <PullRequestListSkeleton />}

        {state.status === 'failed' && (
          <ReviewFailureCard
            failure={state.failure}
            onConnect={connectProvider}
            onRetry={load}
          />
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
                <PullRequestRow
                  key={`${pullRequest.head_fetch_spec}-${pullRequest.number}`}
                  pullRequest={pullRequest}
                />
              ))}
            </div>
          ))}
      </div>
    </div>
  );
}

export default CodeReviewView;
