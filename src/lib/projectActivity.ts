import { segmentFor } from './pipelineFilter';
import { runStatusMeta } from './runStatus';
import type { FeatureStatusCount } from '../types';

/** Per-project counts in the bands `segmentFor` assigns. */
export interface ProjectActivity {
  /** Moving on its own: queued, bootstrapping and verifying as well as running. */
  active: number;
  /** Stopped until a human acts: gates, but also `interrupted` and `needs-credentials`. */
  needsYou: number;
}

export type ProjectActivityMap = Record<string, ProjectActivity>;

const NO_ACTIVITY: ProjectActivity = { active: 0, needsYou: 0 };

function sameCounts(folded: ProjectActivityMap, previous: ProjectActivityMap): boolean {
  const ids = Object.keys(folded);
  if (ids.length !== Object.keys(previous).length) return false;
  return ids.every((id) => {
    const before = previous[id];
    if (before === undefined) return false;
    const after = folded[id];
    return after.active === before.active && after.needsYou === before.needsYou;
  });
}

export function foldProjectActivity(
  rows: FeatureStatusCount[],
  previous?: ProjectActivityMap,
): ProjectActivityMap {
  const folded: ProjectActivityMap = {};

  for (const row of rows) {
    let entry = folded[row.project_id];
    if (entry === undefined) {
      entry = { active: 0, needsYou: 0 };
      folded[row.project_id] = entry;
    }
    const band = segmentFor({ status: row.status });
    if (band === 'needs-you') entry.needsYou += row.count;
    else if (band === 'active') entry.active += row.count;
  }

  if (previous !== undefined && sameCounts(folded, previous)) return previous;
  return folded;
}

export function activityFor(activity: ProjectActivityMap, projectId: string): ProjectActivity {
  return activity[projectId] ?? NO_ACTIVITY;
}

/** Summed over several projects — the ones the collapsed rail has no room to show. */
export function combinedActivity(activity: ProjectActivityMap, projectIds: readonly string[]): ProjectActivity {
  const total = { active: 0, needsYou: 0 };
  for (const id of projectIds) {
    const entry = activityFor(activity, id);
    total.active += entry.active;
    total.needsYou += entry.needsYou;
  }
  return total;
}

export type RailProjectStatus = 'error' | 'bootstrapping' | 'gated' | 'running' | 'idle';

export function railProjectStatus(projectStatus: string, activity: ProjectActivity): RailProjectStatus {
  if (projectStatus === 'error') return 'error';
  if (projectStatus === 'bootstrapping') return 'bootstrapping';
  if (activity.needsYou > 0) return 'gated';
  if (activity.active > 0) return 'running';
  return 'idle';
}

/** `idle` and `error` are the project's own states, not a run's, so neither borrows a run word. */
const RAIL_PROJECT_LABEL: Record<RailProjectStatus, string> = {
  error: 'Error',
  bootstrapping: runStatusMeta('bootstrapping').label,
  gated: runStatusMeta('gated').label,
  running: runStatusMeta('running').label,
  idle: 'Ready',
};

export function railProjectLabel(status: RailProjectStatus): string {
  return RAIL_PROJECT_LABEL[status];
}

export function activityTitle(activity: ProjectActivity): string {
  const parts: string[] = [];
  if (activity.active > 0) parts.push(`${activity.active} active`);
  if (activity.needsYou > 0) parts.push(`${activity.needsYou} needs you`);
  return parts.join(' · ');
}
