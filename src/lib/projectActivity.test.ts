import { describe, expect, it } from 'vitest';

import {
  activityFor,
  activityTitle,
  combinedActivity,
  foldProjectActivity,
  railProjectLabel,
  railProjectStatus,
  type ProjectActivity,
  type ProjectActivityMap,
} from './projectActivity';
import { segmentFor } from './pipelineFilter';
import { RUN_STATUSES } from './runStatus';
import type { FeatureStatusCount } from '../types';

function row(project_id: string, status: string, count = 1): FeatureStatusCount {
  return { project_id, status, count };
}

function bandOf(status: string): ProjectActivity {
  return activityFor(foldProjectActivity([row('p1', status, 1)]), 'p1');
}

describe('foldProjectActivity vocabulary', () => {
  it.each(['gated', 'awaiting_gate', 'parked'])('counts %s toward needsYou and not active', (status) => {
    expect(bandOf(status)).toEqual({ active: 0, needsYou: 1 });
  });

  it('counts bootstrapping toward active and not needsYou', () => {
    expect(bandOf('bootstrapping')).toEqual({ active: 1, needsYou: 0 });
  });

  it.each(['running', 'verifying'])('counts %s toward active', (status) => {
    expect(bandOf(status)).toEqual({ active: 1, needsYou: 0 });
  });

  it.each(['completed', 'published', 'failed'])('counts %s toward neither band', (status) => {
    expect(bandOf(status)).toEqual({ active: 0, needsYou: 0 });
  });

  // The rollup groups on status alone, so this is the assumption that lets it:
  // a PR, live or not, never moves a feature to a different band.
  it.each(RUN_STATUSES)('bands %s the same whether or not it carries a PR', (status) => {
    for (const mr_state of ['draft', 'open', 'merged', 'closed']) {
      expect(segmentFor({ status, mr_url: 'https://x/pr/1', mr_state })).toBe(segmentFor({ status }));
    }
  });

  it('counts an unrecognised status toward neither band without throwing', () => {
    expect(() => bandOf('wibble')).not.toThrow();
    expect(bandOf('wibble')).toEqual({ active: 0, needsYou: 0 });
  });
});

describe('foldProjectActivity', () => {
  it('keys the fold by project id with the right per-project counts', () => {
    const folded = foldProjectActivity([
      row('p1', 'running', 2),
      row('p1', 'gated', 1),
      row('p1', 'completed', 5),
      row('p2', 'parked', 3),
      row('p2', 'verifying', 1),
    ]);

    expect(folded).toEqual({
      p1: { active: 2, needsYou: 1 },
      p2: { active: 1, needsYou: 3 },
    });
  });

  it('sums several rows of the same band', () => {
    expect(foldProjectActivity([row('p1', 'running', 2), row('p1', 'verifying', 3)])).toEqual({
      p1: { active: 5, needsYou: 0 },
    });
  });

  it('yields an empty map for an empty rollup', () => {
    expect(foldProjectActivity([])).toEqual({});
  });

  it('returns the previous map itself when the counts are unchanged', () => {
    const previous = foldProjectActivity([row('p1', 'running', 2), row('p2', 'gated', 1)]);
    const refetched = foldProjectActivity([row('p2', 'gated', 1), row('p1', 'running', 2)], previous);

    expect(refetched).toBe(previous);
  });

  it('returns a new map when a count moved', () => {
    const previous = foldProjectActivity([row('p1', 'running', 2)]);

    expect(foldProjectActivity([row('p1', 'running', 3)], previous)).not.toBe(previous);
    expect(foldProjectActivity([row('p1', 'running', 3)], previous)).toEqual({
      p1: { active: 3, needsYou: 0 },
    });
  });

  it('returns a new map when a project appeared or disappeared', () => {
    const previous = foldProjectActivity([row('p1', 'running', 1)]);

    expect(foldProjectActivity([row('p1', 'running', 1), row('p2', 'gated', 1)], previous)).not.toBe(previous);
    expect(foldProjectActivity([], previous)).not.toBe(previous);
  });

  it('does not treat a same-sized map with different keys as unchanged', () => {
    const previous = foldProjectActivity([row('p1', 'running', 1)]);

    expect(foldProjectActivity([row('p2', 'running', 1)], previous)).not.toBe(previous);
  });
});

describe('activityFor', () => {
  it('reads zeros for a project the rollup never mentioned', () => {
    expect(activityFor({}, 'missing')).toEqual({ active: 0, needsYou: 0 });
  });

  it('reads the folded entry when there is one', () => {
    const folded: ProjectActivityMap = foldProjectActivity([row('p1', 'gated', 4)]);
    expect(activityFor(folded, 'p1')).toEqual({ active: 0, needsYou: 4 });
  });
});

describe('railProjectStatus', () => {
  const quiet: ProjectActivity = { active: 0, needsYou: 0 };
  const busy: ProjectActivity = { active: 3, needsYou: 2 };

  it('keeps error regardless of activity', () => {
    expect(railProjectStatus('error', quiet)).toBe('error');
    expect(railProjectStatus('error', busy)).toBe('error');
  });

  it('keeps bootstrapping regardless of activity', () => {
    expect(railProjectStatus('bootstrapping', quiet)).toBe('bootstrapping');
    expect(railProjectStatus('bootstrapping', busy)).toBe('bootstrapping');
  });

  it('derives gated when anything needs the user', () => {
    expect(railProjectStatus('idle', { active: 4, needsYou: 1 })).toBe('gated');
  });

  it('derives running when work is moving and nothing needs the user', () => {
    expect(railProjectStatus('idle', { active: 4, needsYou: 0 })).toBe('running');
  });

  it('derives idle when nothing is happening', () => {
    expect(railProjectStatus('idle', quiet)).toBe('idle');
  });
});

describe('railProjectLabel', () => {
  it('labels a quiet project without borrowing a run vocabulary word', () => {
    expect(railProjectLabel('idle')).toBe('Ready');
  });

  it('labels a project whose own status is error without the run word Failed', () => {
    expect(railProjectLabel('error')).toBe('Error');
  });

  it('labels the statuses the run vocabulary shares through runStatusMeta', () => {
    expect(railProjectLabel('bootstrapping')).toBe('Bootstrapping');
    expect(railProjectLabel('gated')).toBe('Gate needs you');
    expect(railProjectLabel('running')).toBe('Running');
  });
});

describe('activityTitle', () => {
  it('reads both counts in one line', () => {
    expect(activityTitle({ active: 2, needsYou: 1 })).toBe('2 active · 1 needs you');
  });

  it('names only the count it has', () => {
    expect(activityTitle({ active: 2, needsYou: 0 })).toBe('2 active');
    expect(activityTitle({ active: 0, needsYou: 1 })).toBe('1 needs you');
  });

  it('says nothing when nothing is happening', () => {
    expect(activityTitle({ active: 0, needsYou: 0 })).toBe('');
  });
});

describe('combinedActivity', () => {
  it('sums the named projects and ignores the rest', () => {
    const activity: ProjectActivityMap = {
      p1: { active: 1, needsYou: 0 },
      p2: { active: 2, needsYou: 1 },
      p3: { active: 5, needsYou: 5 },
    };

    expect(combinedActivity(activity, ['p1', 'p2', 'p-quiet'])).toEqual({ active: 3, needsYou: 1 });
  });

  it('is quiet for no projects at all', () => {
    expect(combinedActivity({ p1: { active: 1, needsYou: 1 } }, [])).toEqual({ active: 0, needsYou: 0 });
  });
});
