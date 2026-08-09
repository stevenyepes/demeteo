import { describe, expect, it } from 'vitest';

import {
  DEFAULT_PIPELINE_FILTER,
  filterPipelines,
  segmentCounts,
  segmentFor,
  type PipelineFilterOptions,
  type PipelineRow,
} from './pipelineFilter';

type Row = PipelineRow & { id: string };

function row(id: string, status: string, created_at: number, over: Partial<Row> = {}): Row {
  return { id, status, created_at, title: id, ...over };
}

function ids(rows: ReadonlyArray<Row>): string[] {
  return rows.map((r) => r.id);
}

function opts(over: Partial<PipelineFilterOptions> = {}): PipelineFilterOptions {
  return { ...DEFAULT_PIPELINE_FILTER, ...over };
}

describe('segmentFor', () => {
  it.each(['gated', 'awaiting_gate', 'parked', 'needs-credentials', 'needs_credentials', 'interrupted'])(
    'puts %s in needs-you',
    (status) => {
      expect(segmentFor(row('f', status, 1))).toBe('needs-you');
    },
  );

  it.each(['pending', 'running', 'verifying'])('puts %s in active', (status) => {
    expect(segmentFor(row('f', status, 1))).toBe('active');
  });

  // Amber *and* still moving on its own: nothing is waiting on a human yet, so
  // it must not jump the queue ahead of a real gate.
  it('puts bootstrapping in active despite its amber tone', () => {
    expect(segmentFor(row('f', 'bootstrapping', 1))).toBe('active');
  });

  it.each(['completed', 'published', 'awaiting_mr', 'pr_ready', 'failed', 'error', 'over-budget', 'cancelled', 'unreachable'])(
    'puts %s in done',
    (status) => {
      expect(segmentFor(row('f', status, 1))).toBe('done');
    },
  );

  it('treats an unknown status as done rather than throwing or claiming attention', () => {
    expect(segmentFor(row('f', 'sideways_quantum_flux', 1))).toBe('done');
    expect(segmentFor(row('f', '', 1))).toBe('done');
  });

  it('resolves through featureRunStatus, so a published PR wins over the raw status', () => {
    expect(segmentFor(row('f', 'running', 1, { mr_url: 'https://x/pr/1', mr_state: 'open' }))).toBe('done');
  });
});

describe('filterPipelines segments', () => {
  const rows: Row[] = [
    row('run', 'running', 5),
    row('gate', 'gated', 4),
    row('done', 'completed', 3),
    row('cred', 'needs-credentials', 2),
    row('fail', 'failed', 1),
  ];

  it('keeps everything for all', () => {
    expect(ids(filterPipelines(rows, opts({ segment: 'all' })))).toEqual(['gate', 'cred', 'run', 'done', 'fail']);
  });

  it('keeps only the runs waiting on a human for needs-you', () => {
    expect(ids(filterPipelines(rows, opts({ segment: 'needs-you' })))).toEqual(['gate', 'cred']);
  });

  it('keeps only the moving runs for active', () => {
    expect(ids(filterPipelines(rows, opts({ segment: 'active' })))).toEqual(['run']);
  });

  it('keeps the finished runs, good and bad, for done', () => {
    expect(ids(filterPipelines(rows, opts({ segment: 'done' })))).toEqual(['done', 'fail']);
  });

  it('returns an empty list for an empty input', () => {
    const empty: Row[] = [];
    expect(filterPipelines(empty, opts({ segment: 'needs-you' }))).toEqual([]);
    expect(filterPipelines(empty, opts())).toBe(empty);
  });
});

describe('filterPipelines ordering', () => {
  it('bands needs-you first, then active, then the rest, newest first inside each band', () => {
    const rows: Row[] = [
      row('done-old', 'completed', 10),
      row('active-new', 'running', 70),
      row('gate-old', 'gated', 20),
      row('done-new', 'failed', 80),
      row('active-old', 'pending', 30),
      row('gate-new', 'parked', 90),
    ];

    expect(ids(filterPipelines(rows, opts()))).toEqual([
      'gate-new',
      'gate-old',
      'active-new',
      'active-old',
      'done-new',
      'done-old',
    ]);
  });

  it('is stable — equal created_at inside a band keeps input order', () => {
    const rows: Row[] = [
      row('c', 'completed', 1),
      row('a', 'gated', 1),
      row('b', 'gated', 1),
      row('d', 'completed', 1),
    ];

    expect(ids(filterPipelines(rows, opts()))).toEqual(['a', 'b', 'c', 'd']);
  });

  it('ignores bands for newest and oldest', () => {
    const rows: Row[] = [
      row('mid', 'completed', 2),
      row('new', 'running', 3),
      row('old', 'gated', 1),
    ];

    expect(ids(filterPipelines(rows, opts({ sort: 'newest' })))).toEqual(['new', 'mid', 'old']);
    expect(ids(filterPipelines(rows, opts({ sort: 'oldest' })))).toEqual(['old', 'mid', 'new']);
  });
});

describe('filterPipelines query', () => {
  const rows: Row[] = [
    row('a', 'running', 3, { title: 'Add SSH keepalive', description: 'Pooled connections die silently' }),
    row('b', 'running', 2, { title: 'Windows path fence', description: 'Join with PathBuf instead' }),
    row('c', 'running', 1, { title: 'Runner mirror', description: null }),
  ];

  it('matches title and description case-insensitively', () => {
    expect(ids(filterPipelines(rows, opts({ query: 'ssh' })))).toEqual(['a']);
    expect(ids(filterPipelines(rows, opts({ query: 'PATHBUF' })))).toEqual(['b']);
  });

  it('requires every whitespace-separated term, in any order', () => {
    expect(ids(filterPipelines(rows, opts({ query: 'silently keepalive' })))).toEqual(['a']);
    expect(ids(filterPipelines(rows, opts({ query: 'ssh pathbuf' })))).toEqual([]);
  });

  // The fuzzy subsequence matcher in ProjectRail would match nearly every
  // description for a short query; substring keeps the filter honest.
  it('does not match a subsequence spread across the text', () => {
    expect(ids(filterPipelines(rows, opts({ query: 'aik' })))).toEqual([]);
  });

  it('ignores a blank or whitespace-only query', () => {
    expect(filterPipelines(rows, opts({ query: '   ' }))).toBe(rows);
  });

  it('survives a row with no description', () => {
    expect(ids(filterPipelines(rows, opts({ query: 'mirror' })))).toEqual(['c']);
  });
});

describe('filterPipelines identity', () => {
  const ordered: Row[] = [row('gate', 'gated', 3), row('run', 'running', 2), row('done', 'completed', 1)];

  it('returns the input array itself when nothing is dropped or moved', () => {
    expect(filterPipelines(ordered, opts())).toBe(ordered);
  });

  it('returns a new array when the order changes', () => {
    const jumbled: Row[] = [ordered[2], ordered[0], ordered[1]];
    const result = filterPipelines(jumbled, opts());

    expect(result).not.toBe(jumbled);
    expect(ids(result)).toEqual(['gate', 'run', 'done']);
  });

  it('returns a new array when a row is filtered out', () => {
    expect(filterPipelines(ordered, opts({ segment: 'needs-you' }))).not.toBe(ordered);
  });

  it('does not mutate the input', () => {
    const jumbled: Row[] = [ordered[2], ordered[0], ordered[1]];
    filterPipelines(jumbled, opts());

    expect(ids(jumbled)).toEqual(['done', 'gate', 'run']);
  });
});

describe('segmentCounts', () => {
  it('counts every band plus the unfiltered total', () => {
    const rows: Row[] = [
      row('gate', 'gated', 5),
      row('creds', 'needs-credentials', 4),
      row('run', 'running', 3),
      row('boot', 'bootstrapping', 2),
      row('done', 'completed', 1),
    ];

    expect(segmentCounts(rows)).toEqual({ all: 5, 'needs-you': 2, active: 2, done: 1 });
  });

  it('agrees with segmentFor rather than keeping its own status table', () => {
    const rows: Row[] = [
      row('a', 'gated', 4),
      row('b', 'verifying', 3),
      row('c', 'failed', 2),
      row('d', 'bootstrapping', 1),
    ];
    const counts = segmentCounts(rows);

    for (const band of ['needs-you', 'active', 'done'] as const) {
      expect(counts[band]).toBe(rows.filter((r) => segmentFor(r) === band).length);
    }
  });

  it('reports zeros for an empty list', () => {
    expect(segmentCounts([])).toEqual({ all: 0, 'needs-you': 0, active: 0, done: 0 });
  });
});
