import type {
  CacheSweepEntry,
  CacheSweepReport,
  CacheSweepVerdict,
} from "./featureDetail";

const VERDICT_ORDER: readonly CacheSweepVerdict[] = ["delete", "unknown", "keep"];

export interface SweepVerdictGroup {
  verdict: CacheSweepVerdict;
  entries: CacheSweepEntry[];
}

export interface SweepProjectView {
  projectId: string;
  name: string;
  cloneDir: string | null;
  error: string | null;
  worktreeListError: string | null;
  /** Non-empty groups only, in `delete`, `unknown`, `keep` order. */
  groups: SweepVerdictGroup[];
}

export interface SweepTotals {
  delete: number;
  unknown: number;
  keep: number;
  deleted: number;
  failed: number;
  spared: number;
  /** Projects with an `error` or a `worktree_list_error`. */
  projectErrors: number;
}

export interface SweepView {
  projects: SweepProjectView[];
  totals: SweepTotals;
}

export function viewSweep(
  report: CacheSweepReport,
  projectNames: Readonly<Record<string, string>>,
): SweepView {
  const totals: SweepTotals = {
    delete: 0,
    unknown: 0,
    keep: 0,
    deleted: 0,
    failed: 0,
    spared: 0,
    projectErrors: 0,
  };
  const projects = report.projects.map((project): SweepProjectView => {
    if (project.error || project.worktree_list_error) totals.projectErrors++;
    for (const entry of project.entries) {
      totals[entry.verdict]++;
      if (entry.outcome) totals[entry.outcome.status]++;
    }
    return {
      projectId: project.project_id,
      name: projectNames[project.project_id] ?? project.project_id,
      cloneDir: project.clone_dir,
      error: project.error,
      worktreeListError: project.worktree_list_error,
      groups: VERDICT_ORDER.map((verdict) => ({
        verdict,
        entries: project.entries.filter((e) => e.verdict === verdict),
      })).filter((g) => g.entries.length > 0),
    };
  });
  return { projects, totals };
}
