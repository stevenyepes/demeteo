/**
 * Everything one pull-request row displays, decided here so the decisions can
 * be asserted without mounting a row.
 *
 * The three tiers the row renders — what you scan, what gives it context, what
 * you read once you have stopped — are three groups of fields on one object
 * rather than three helpers, because the choice a tier makes is *whether it has
 * anything to say*. A provider list endpoint that omits the diffstat must cost
 * the row that clause and nothing else, and a `0` inferred from an absent field
 * is the one wrong answer available: it reads as a pull request that changes
 * nothing.
 */

import type { PullRequestSummary } from './pullRequests';
import { runStatusMeta, type RunStatusTone } from './runStatus';
import { relativeTime } from './utils';

export interface RowChip {
  label: string;
  tone: RunStatusTone;
}

export interface RowDiffstat {
  additions: string;
  deletions: string;
}

export interface PullRequestRowModel {
  number: string;
  title: string;
  url: string;
  chips: RowChip[];
  /** Scan tier: age of the most recent activity, or `null` for a timestamp
   *  the provider spelled in something `Date.parse` does not read. */
  updatedAgo: string | null;
  branchLabel: string;
  author: string;
  diffstat: RowDiffstat | null;
  fileLabel: string | null;
  /** Detail tier, already joined: `opened 3d ago · updated 2h ago`. */
  timeline: string;
}

export function describePullRequestRow(
  pr: PullRequestSummary,
  now: number = Date.now(),
): PullRequestRowModel {
  const opened = ageOf(pr.created_at, now);
  const updated = ageOf(pr.updated_at, now);

  return {
    number: `#${pr.number}`,
    title: pr.title,
    url: pr.web_url,
    chips: chipsFor(pr),
    updatedAgo: updated,
    branchLabel: `${pr.source_branch} → ${pr.target_branch}`,
    author: pr.author,
    diffstat: diffstatOf(pr),
    fileLabel: fileLabelOf(pr.changed_files),
    timeline: [opened && `opened ${opened}`, updated && `updated ${updated}`]
      .filter((clause): clause is string => clause !== null && clause !== '')
      .join(' · '),
  };
}

function chipsFor(pr: PullRequestSummary): RowChip[] {
  const chips: RowChip[] = [];
  if (pr.draft) chips.push({ label: 'Draft', tone: 'slate' });
  // Three readings of one field: a verdict, the absence of one, and no claim.
  // `false` says nothing on purpose — a queue where every clean row carries a
  // reassurance is a queue nobody scans.
  if (pr.has_conflicts === true) chips.push({ label: 'Conflicts', tone: 'amber' });
  else if (pr.has_conflicts === null) {
    chips.push({ label: 'Merge unknown', tone: 'slate' });
  }

  if (pr.review_status) {
    const meta = runStatusMeta(pr.review_status);
    chips.push({ label: meta.label, tone: meta.tone });
  }
  return chips;
}

function diffstatOf(pr: PullRequestSummary): RowDiffstat | null {
  if (typeof pr.additions !== 'number' && typeof pr.deletions !== 'number') return null;
  return {
    additions: `+${pr.additions ?? 0}`,
    deletions: `−${pr.deletions ?? 0}`,
  };
}

function fileLabelOf(changed: number | null | undefined): string | null {
  if (typeof changed !== 'number') return null;
  return changed === 1 ? '1 file' : `${changed} files`;
}

function ageOf(timestamp: string, now: number): string | null {
  const ms = Date.parse(timestamp);
  return Number.isNaN(ms) ? null : relativeTime(ms, now);
}
