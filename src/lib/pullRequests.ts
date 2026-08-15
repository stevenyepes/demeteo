/**
 * Open pull requests for a project, and the four ways reading them can fail.
 *
 * The failure union is the point of this module. A listing that renders every
 * failure as an empty list tells the user "nothing is waiting for review" when
 * the truth is "your token expired" — the one reading that makes them stop
 * looking. So the states are distinct values here, each with the fact the
 * message needs (which host, which status), and the view is left with no way
 * to collapse them into a blank page.
 *
 * A rejection this module cannot recognise becomes `api-error` carrying the
 * provider's own words rather than a summary of them: a status Demeteo has
 * never seen is exactly the case where paraphrasing costs the user the only
 * evidence they have.
 */

import { invoke } from '@tauri-apps/api/core';

import { formatError } from './errors';

/**
 * One open pull request or merge request.
 *
 * The identity half mirrors the Rust `MrSummary` field for field, in its
 * snake_case IPC spelling. The review tier below it is nullable because
 * neither provider's *list* endpoint carries it: GitHub reports
 * additions/deletions/changed_files only from the single-PR GET, GitLab
 * computes mergeability asynchronously and answers `null` while it does. A row
 * renders the tier it was handed, never a zero it inferred from an absence.
 */
export interface PullRequestSummary {
  number: number;
  title: string;
  author: string;
  source_branch: string;
  target_branch: string;
  draft: boolean;
  web_url: string;
  created_at: string;
  updated_at: string;
  head_repo_path: string | null;
  head_fetch_spec: string;
  from_fork: boolean;
  maintainer_can_modify: boolean;
  head_repo_push: boolean;
  additions?: number | null;
  deletions?: number | null;
  changed_files?: number | null;
  has_conflicts?: boolean | null;
  /** Status of the Demeteo run reviewing this request, once one exists — a
   *  `lib/runStatus.ts` status string, so the chip resolves through the same
   *  vocabulary every other run surface uses. */
  review_status?: string | null;
}

export type PullRequestListFailure =
  | { kind: 'no-provider' }
  | { kind: 'token-rejected'; provider: string; host: string; status: number }
  | { kind: 'rate-limited'; host: string }
  | { kind: 'api-error'; host: string; status: number | null; body: string };

/** Every open pull request across the project's repositories. */
export async function listOpenPullRequests(projectId: string): Promise<PullRequestSummary[]> {
  return invoke<PullRequestSummary[]>('list_open_pull_requests', { projectId });
}

const UNNAMED_HOST = 'The provider';

/** Rejections arrive as a JSON string (`Result<T, String>`) or as an already
 *  decoded object, and both spellings mean the same thing to the caller. */
export function asPullRequestListFailure(err: unknown): PullRequestListFailure {
  const payload = decodePayload(err);
  const host = readString(payload, 'host') ?? UNNAMED_HOST;

  switch (payload?.kind) {
    case 'no-provider':
      return { kind: 'no-provider' };
    case 'token-rejected':
      return {
        kind: 'token-rejected',
        provider: readString(payload, 'provider') ?? 'provider',
        host,
        status: readNumber(payload, 'status') ?? 401,
      };
    case 'rate-limited':
      return { kind: 'rate-limited', host };
    default:
      return {
        kind: 'api-error',
        host,
        status: readNumber(payload, 'status'),
        body: readString(payload, 'body') ?? formatError(err),
      };
  }
}

function decodePayload(err: unknown): Record<string, unknown> | null {
  if (typeof err === 'string') {
    try {
      const parsed: unknown = JSON.parse(err);
      return typeof parsed === 'object' && parsed !== null
        ? (parsed as Record<string, unknown>)
        : null;
    } catch {
      return null;
    }
  }
  if (typeof err === 'object' && err !== null) return err as Record<string, unknown>;
  return null;
}

function readString(payload: Record<string, unknown> | null, key: string): string | null {
  const value = payload?.[key];
  return typeof value === 'string' && value.length > 0 ? value : null;
}

function readNumber(payload: Record<string, unknown> | null, key: string): number | null {
  const value = payload?.[key];
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

export interface FailureAction {
  /** Connecting and reconnecting land on the same manager, so they are one
   *  intent wearing two labels; `retry` re-runs the fetch in place. */
  intent: 'connect' | 'retry';
  label: string;
  primary?: boolean;
}

export interface FailureCopy {
  title: string;
  body: string;
  /** The provider's response, verbatim and capped. Rendered as a mono block. */
  detail?: string;
  actions: FailureAction[];
}

const PROVIDER_NAMES: Record<string, string> = {
  github: 'GitHub',
  gitlab: 'GitLab',
};

/** `GitHub`, not `Github`: a provider that renders its own name wrong reads as
 *  a page that does not know which provider it is talking to. */
export function providerName(kind: string): string {
  return PROVIDER_NAMES[kind.toLowerCase()] ?? kind;
}

const DETAIL_LIMIT = 600;

export function truncateDetail(body: string, limit: number = DETAIL_LIMIT): string {
  const trimmed = body.trim();
  return trimmed.length <= limit ? trimmed : `${trimmed.slice(0, limit)}…`;
}

export function describeListFailure(failure: PullRequestListFailure): FailureCopy {
  switch (failure.kind) {
    case 'no-provider':
      return {
        title: 'No provider connected',
        body: "This project's repositories aren't mapped to a GitHub or GitLab connection, so Demeteo can't list pull requests. Connect one and it will read them with your token.",
        actions: [{ intent: 'connect', label: 'Connect a provider', primary: true }],
      };
    case 'token-rejected': {
      const provider = providerName(failure.provider);
      return {
        title: `Your ${provider} token was rejected`,
        body: `${failure.host} answered ${failure.status}. The token stored in your keyring is missing, expired, or lost the scope that reads pull requests — reconnect the provider to replace it.`,
        actions: [
          { intent: 'connect', label: `Reconnect ${provider}`, primary: true },
          { intent: 'retry', label: 'Retry' },
        ],
      };
    }
    case 'rate-limited':
      return {
        title: `${failure.host} is rate-limiting this token`,
        body: 'The pull-request list will be readable again shortly. Nothing was lost — reviews already running are unaffected.',
        actions: [{ intent: 'retry', label: 'Retry' }],
      };
    case 'api-error':
      return {
        title: "Couldn't read pull requests",
        body: `${failure.host} answered ${failure.status ?? 'with an error'}. This is the provider's response, unchanged:`,
        detail: truncateDetail(failure.body),
        actions: [{ intent: 'retry', label: 'Retry', primary: true }],
      };
  }
}
