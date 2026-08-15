import type { ReactElement } from 'react';
import { AlertTriangle } from 'lucide-react';

import { describeListFailure, type PullRequestListFailure } from '../../lib/pullRequests';

export interface ReviewFailureCardProps {
  failure: PullRequestListFailure;
  onConnect: () => void;
  onRetry: () => void;
}

/**
 * The four ways the listing can fail, each wearing its own words
 * (`lib/pullRequests.ts` holds them). This is a card and not a toast because a
 * failure here *is* the page: dismissing it would leave an empty list behind,
 * which is the reading the whole failure union exists to prevent.
 */
export function ReviewFailureCard({
  failure,
  onConnect,
  onRetry,
}: ReviewFailureCardProps): ReactElement {
  const copy = describeListFailure(failure);

  return (
    <div
      role="alert"
      data-testid="code-review-failure"
      data-failure={failure.kind}
      className="glass-panel rounded-2xl border border-amber-500/20 p-6 space-y-4"
    >
      <div className="flex items-start gap-3">
        <AlertTriangle aria-hidden="true" className="mt-0.5 w-5 h-5 shrink-0 text-amber-400" />
        <div className="min-w-0 space-y-2">
          <h2 className="font-heading text-base font-semibold text-white">{copy.title}</h2>
          <p className="text-sm text-slate-400 leading-relaxed">{copy.body}</p>
        </div>
      </div>

      {copy.detail && (
        <pre
          data-testid="code-review-failure-detail"
          className="max-h-40 overflow-y-auto rounded-lg border border-white/5 bg-black/40 p-3 font-mono text-[11px] text-slate-300 whitespace-pre-wrap break-words"
        >
          {copy.detail}
        </pre>
      )}

      <div className="flex flex-wrap gap-3">
        {copy.actions.map((action) => (
          <button
            key={action.label}
            type="button"
            onClick={action.intent === 'connect' ? onConnect : onRetry}
            className={`rounded-lg px-4 py-2 text-sm font-medium transition-all ${
              action.primary
                ? 'bg-violet-600 hover:bg-violet-500 text-white'
                : 'border border-white/10 bg-white/5 hover:bg-white/10 text-slate-200'
            }`}
          >
            {action.label}
          </button>
        ))}
      </div>
    </div>
  );
}

export default ReviewFailureCard;
