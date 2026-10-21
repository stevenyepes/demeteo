import { useCallback, useState, type ReactElement } from 'react';
import { ExternalLink, MessageSquarePlus } from 'lucide-react';

import { Button } from '../ui/Button';
import { Modal } from '../ui/Modal';
import { formatError } from '../../lib/errors';
import { postPullRequestComment } from '../../lib/pullRequests';

export interface PostReviewCommentProps {
  projectId: string;
  /** The pull request the report is about, as the provider addresses it. */
  pullRequestUrl: string;
  /** What the confirmation calls it — `PR #412`, `!9`. Rendered into a
   *  sentence, so it is a name and not a URL. */
  pullRequestLabel: string;
  /** The report, as the agent wrote it. The attribution is the backend's. */
  report: string;
}

type Stage =
  | { kind: 'idle' }
  | { kind: 'confirming' }
  | { kind: 'posting' }
  | { kind: 'posted'; commentUrl: string }
  | { kind: 'failed'; message: string };

const BUTTON = 'Post to the pull request';
const CONFIRM_ACTION = 'Post comment';

/**
 * Publishes a finished review report to the pull request it is about — after
 * asking, and never on its own.
 *
 * Everything else a run produces stays inside Demeteo, where a wrong answer
 * costs a re-run. This one crosses into a service Demeteo does not own and
 * lands under the token owner's name, where the provider offers no delete this
 * app is wired for and every subscriber has already been mailed a copy. So the
 * irreversibility is spent by a person, on a second click, against copy that
 * names who will see it — and the created comment's URL is reported back,
 * because "posted" without an address leaves the user opening the pull request
 * to find out what was sent in their name.
 *
 * A second post is a second comment, not an edit. The button therefore retires
 * once it has succeeded rather than returning to idle: a control that looks
 * ready to press again is the one shape that turns one irreversible action into
 * two.
 */
export function PostReviewComment({
  projectId,
  pullRequestUrl,
  pullRequestLabel,
  report,
}: PostReviewCommentProps): ReactElement {
  const [stage, setStage] = useState<Stage>({ kind: 'idle' });

  const post = useCallback(() => {
    setStage({ kind: 'posting' });
    postPullRequestComment({ projectId, pullRequestUrl, body: report })
      .then((commentUrl) => setStage({ kind: 'posted', commentUrl }))
      .catch((err: unknown) => setStage({ kind: 'failed', message: formatError(err) }));
  }, [projectId, pullRequestUrl, report]);

  if (stage.kind === 'posted') {
    return (
      <div className="flex flex-wrap items-center gap-2 text-xs text-slate-300">
        <span>Posted on {pullRequestLabel}.</span>
        <a
          href={stage.commentUrl}
          target="_blank"
          rel="noreferrer"
          data-testid="posted-comment-link"
          className="flex items-center gap-1 text-cyan-400 transition-colors hover:text-cyan-300"
        >
          View the comment
          <ExternalLink className="h-3 w-3" aria-hidden="true" />
        </a>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <Button
        data-testid="post-review-comment"
        disabled={report.trim().length === 0 || stage.kind === 'posting'}
        onClick={() => setStage({ kind: 'confirming' })}
      >
        <MessageSquarePlus className="h-4 w-4" aria-hidden="true" />
        {stage.kind === 'posting' ? 'Posting…' : BUTTON}
      </Button>

      {stage.kind === 'failed' && (
        <p data-testid="post-review-failed" className="text-xs leading-relaxed text-ruby-400">
          {stage.message}
        </p>
      )}

      {stage.kind === 'confirming' && (
        <Modal
          onClose={() => setStage({ kind: 'idle' })}
          className="raised w-full max-w-md p-6"
        >
          <h2 className="font-heading text-base font-semibold text-white">{BUTTON}</h2>
          <p className="mt-3 text-sm leading-relaxed text-slate-300">
            This posts the review report as a comment on {pullRequestLabel}, using your provider
            token. It will be visible to everyone with access to the repository.
          </p>
          <div className="mt-6 flex justify-end gap-2">
            <Button onClick={() => setStage({ kind: 'idle' })}>Cancel</Button>
            <Button variant="primary" data-testid="confirm-post-review-comment" onClick={post}>
              {CONFIRM_ACTION}
            </Button>
          </div>
        </Modal>
      )}
    </div>
  );
}

export default PostReviewComment;
