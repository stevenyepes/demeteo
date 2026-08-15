/**
 * One open pull request → the arguments that start a review run on it.
 *
 * Pure, and a plan is a *value* the row renders rather than a call it makes,
 * because the interesting answer is the refusal. `head_fetch_spec` crosses IPC
 * as a plain string — the Rust `Refspec` newtype does not survive serde on the
 * way out — so nothing here may treat what arrived as something git has already
 * accepted. A summary that lost it must not launch with an empty spec: the
 * origin would carry `fetch_spec: ""`, the cut would land on whatever else
 * resolved, and the reviewer would be handed the `git diff ..HEAD` empty range
 * that `domain/review_base.rs` exists to keep out of a prompt. An empty diff
 * reads as a finished review of a clean change.
 *
 * The `refs/` requirement is `FeatureOrigin::fetch_plan`'s, restated here so a
 * summary that cannot be reviewed says so on the row, instead of failing a run
 * that already has a worktree.
 */

import type { FeatureOrigin } from '../types';
import type { PullRequestSummary } from './pullRequests';

/** The bundled review starter (`src-tauri/workflows/code-review.json`). Its id
 *  is stable across edits — `workflow_revert_to_default` republishes onto the
 *  same row — which is what lets this name one workflow outright. */
export const REVIEW_STARTER_WORKFLOW_ID = 'wf-starter-code-review';

/** A subset of `LaunchRunParams`, so a plan reaches `useLaunchRun` unchanged
 *  rather than being copied field by field into it. */
export interface ReviewLaunchParams {
  workflowId: string;
  title: string;
  description: string;
  origin: FeatureOrigin;
  diffBaseBranch: string;
}

export type ReviewLaunchPlan =
  | { ok: true; launch: ReviewLaunchParams }
  | { ok: false; reason: 'no-head-ref' | 'no-base-branch'; message: string };

const TITLE_LIMIT = 72;

export function planReviewLaunch(
  pullRequest: PullRequestSummary,
  instructions = '',
): ReviewLaunchPlan {
  const fetchSpec = usableFetchSpec(pullRequest.head_fetch_spec);
  if (fetchSpec === null) {
    return {
      ok: false,
      reason: 'no-head-ref',
      message:
        'This pull request arrived without a ref Demeteo can fetch its head from, so there is no commit to review.',
    };
  }

  const diffBaseBranch = named(pullRequest.target_branch);
  if (diffBaseBranch === null) {
    return {
      ok: false,
      reason: 'no-base-branch',
      message:
        'This pull request names no target branch, so the review would have no range to measure the change against.',
    };
  }

  return {
    ok: true,
    launch: {
      workflowId: REVIEW_STARTER_WORKFLOW_ID,
      title: reviewTitle(pullRequest),
      description: reviewDescription(pullRequest, instructions),
      origin: {
        kind: 'ref',
        fetch_spec: fetchSpec,
        label: named(pullRequest.source_branch) ?? `PR #${pullRequest.number}`,
      },
      diffBaseBranch,
    },
  };
}

/** The `refs/` prefix subsumes the leading-`-` half of the Rust `Refspec`
 *  check: a value starting with it cannot also start with an option marker. */
function usableFetchSpec(spec: string | null | undefined): string | null {
  if (typeof spec !== 'string' || !spec.startsWith('refs/')) return null;
  return /[\s\p{Cc}]/u.test(spec) ? null : spec;
}

function named(value: string | null | undefined): string | null {
  const trimmed = value?.trim() ?? '';
  return trimmed.length > 0 ? trimmed : null;
}

/**
 * `Review PR #412 — Tighten the refspec guard`, capped at 72 characters.
 *
 * The number is the only part that identifies the run in a pipeline list, so it
 * survives every truncation and the provider's title is the half that gives
 * way. Cut at a word boundary: a title severed mid-word reads as a corrupted
 * row rather than a shortened one.
 */
function reviewTitle(pullRequest: PullRequestSummary): string {
  const stem = `Review PR #${pullRequest.number}`;
  const title = named(pullRequest.title);
  if (title === null) return stem;

  const prefix = `${stem} — `;
  const full = `${prefix}${title}`;
  if (full.length <= TITLE_LIMIT) return full;

  const room = TITLE_LIMIT - prefix.length - 1;
  if (room <= 0) return stem;

  const head = title.slice(0, room);
  const boundary = head.lastIndexOf(' ');
  const kept = (boundary > 0 ? head.slice(0, boundary) : head).replace(/[\s.,;:—-]+$/, '');
  return kept.length > 0 ? `${prefix}${kept}…` : stem;
}

/**
 * What the run is about, in the plainest terms available: which request, where
 * it lives, and what the person launching it asked for on top.
 *
 * It renders into `{{feature_description}}`, one line above the review
 * starter's "How to review", and states no criteria for the same reason that
 * template states none.
 */
function reviewDescription(pullRequest: PullRequestSummary, instructions: string): string {
  const title = named(pullRequest.title);
  const author = named(pullRequest.author);
  const head = named(pullRequest.source_branch);
  const base = named(pullRequest.target_branch);

  const identity = [
    title === null
      ? `Pull request #${pullRequest.number}`
      : `Pull request #${pullRequest.number}: ${title}`,
    named(pullRequest.web_url),
    author === null ? null : `Author: ${author}`,
    head === null || base === null
      ? null
      : `Head: ${head}${pullRequest.from_fork ? ' (from a fork)' : ''} → base: ${base}`,
  ].filter((line): line is string => line !== null);

  const extra = named(instructions);
  return extra === null
    ? identity.join('\n')
    : `${identity.join('\n')}\n\nExtra instructions:\n${extra}`;
}
