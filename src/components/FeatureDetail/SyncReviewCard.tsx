import React from 'react';
import { AlertTriangle, GitCompare, Trash2, Upload } from 'lucide-react';
import type { SyncSessionView } from '../../types';
import { TONE_CHIP, TONE_TEXT } from '../../lib/runStatus';

/**
 * A resolution that is committed on the feature branch and has not been
 * published — the state P4 exists to create, and the only surface that can end
 * it.
 *
 * Amber, not emerald: `lib/runStatus.ts` assigns amber to "needs a human", and
 * that is exactly what this is. Emerald would read as "done" beside a merge the
 * open pull request has never seen.
 *
 * **The diff is `head_before..merge_commit_sha`, and nothing else.**
 * `merge_commit_sha^` names the pre-merge tip only while the resolution is a
 * single merge commit; a resolver that added a follow-up commit — which agents
 * do routinely — makes the first parent that follow-up's parent instead, and
 * the review then silently omits the merge itself. The pre-merge tip is
 * persisted at sync time (V43 `head_before`) precisely because it is
 * unrecoverable afterwards, and a session that lacks it says so rather than
 * offering a diff against a guess.
 */
interface SyncReviewCardProps {
  session: SyncSessionView;
  /** Which action is in flight; both buttons disable while either is. */
  pending: 'push' | 'discard' | null;
  onViewDiff: (refs: { baseRef: string; headRef: string }) => void;
  onPush: () => void;
  onDiscard: () => void;
}

const NO_BASE =
  'This sync never recorded where the branch was before the merge, so there is no honest base to diff or reset against.';
const NO_HEAD =
  'This sync recorded no resolution commit, so there is nothing here to identify what would be shown or undone.';

export const SyncReviewCard: React.FC<SyncReviewCardProps> = ({
  session,
  pending,
  onViewDiff,
  onPush,
  onDiscard,
}) => {
  const base = session.head_before;
  const head = session.merge_commit_sha;
  const busy = pending !== null;

  return (
    <div
      data-testid="sync-review"
      className={`px-6 py-3 border-b flex items-start gap-3 ${TONE_CHIP.amber}`}
    >
      <div className="flex-1 space-y-2 text-xs text-slate-200">
        <div className={`flex items-center gap-2 ${TONE_TEXT.amber}`}>
          <AlertTriangle className="w-3.5 h-3.5 shrink-0" />
          <span>
            <strong>Conflicts resolved, not published.</strong> The merge is on{' '}
            <span className="font-mono">{session.feature_branch}</span> and origin has not
            seen it. Read it before it reaches the pull request.
          </span>
        </div>
        <div className="flex flex-wrap justify-end gap-2">
          <button
            type="button"
            onClick={() => (base && head ? onViewDiff({ baseRef: base, headRef: head }) : undefined)}
            disabled={!base || !head}
            title={
              base && head
                ? `Diff ${base.slice(0, 7)}..${head.slice(0, 7)}`
                : base
                  ? NO_HEAD
                  : NO_BASE
            }
            className="flex items-center gap-1.5 px-3 py-1.5 bg-violet-600 hover:bg-violet-500 hover:shadow-[0_0_20px_rgba(139,92,246,0.5)] rounded text-xs font-bold text-white transition disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:shadow-none"
          >
            <GitCompare className="w-3 h-3" />
            View diff
          </button>
          <button
            type="button"
            onClick={onPush}
            disabled={busy || !head}
            title={
              head
                ? 'Push the resolution to origin. Safe to press twice — a resolution already there answers with itself.'
                : NO_HEAD
            }
            className="flex items-center gap-1.5 px-3 py-1.5 bg-emerald-600/20 hover:bg-emerald-600 border border-emerald-500/30 text-emerald-400 hover:text-white rounded text-xs font-bold transition disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <Upload className="w-3 h-3" />
            {pending === 'push' ? 'Pushing…' : 'Push to origin'}
          </button>
          <button
            type="button"
            onClick={onDiscard}
            disabled={busy || !base || !head}
            title={
              base && head
                ? `Move ${session.feature_branch} back to ${base.slice(0, 7)} and abandon this sync. Refused if anything has been committed on top or the checkout is dirty. The conflict is not restored — sync again for a fresh one.`
                : base
                  ? NO_HEAD
                  : NO_BASE
            }
            className="flex items-center gap-1.5 px-3 py-1.5 border border-rose-500/30 hover:bg-rose-500/10 rounded text-xs font-bold text-rose-300 transition disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <Trash2 className="w-3 h-3" />
            {pending === 'discard' ? 'Discarding…' : 'Discard merge'}
          </button>
        </div>
      </div>
    </div>
  );
};
