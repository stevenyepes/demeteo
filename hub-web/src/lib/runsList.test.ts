import { describe, expect, it } from 'vitest';

import type { FleetInstance, NeedsYou } from './fleet';
import type { RunSummary } from './runs';
import { bandRuns, ticketText } from './runsList';

function run(instanceId: string, featureId: string, overrides: Partial<RunSummary> = {}): RunSummary {
  return {
    instanceId,
    featureId,
    status: 'running',
    startedAt: 100,
    updatedAt: 200,
    costCents: null,
    title: null,
    projectId: null,
    projectName: null,
    stepId: null,
    stepTitle: null,
    tickets: { completed: 0, failed: 0, running: 0 },
    ...overrides,
  };
}

function gate(instanceId: string, featureId: string, since: number): NeedsYou {
  return {
    instanceId,
    featureId,
    stepId: 'review',
    stepExecutionId: `se-${featureId}-${since}`,
    since,
    title: null,
    projectName: null,
    stepTitle: 'Review diff',
  };
}

function instance(id: string, label: string | null, needsYou: NeedsYou[] | null): FleetInstance {
  return {
    id,
    label,
    present: true,
    os: 'linux',
    arch: 'x86_64',
    version: '1.4.2',
    scopes: ['read'],
    readGranted: true,
    lastSeen: 0,
    liveRuns: 0,
    needsYou,
    harnesses: null,
    projectCount: null,
  };
}

describe('bandRuns', () => {
  it('bands a run with a waiting gate as needing you, oldest gate first, and the rest by status', () => {
    const fleet = [instance('a', 'workstation', [gate('a', 'f-late', 50), gate('a', 'f-early', 10)])];
    const bands = bandRuns(
      [
        run('a', 'f-late', { updatedAt: 900 }),
        run('a', 'f-early', { updatedAt: 100 }),
        run('a', 'f-live', { updatedAt: 500 }),
        run('a', 'f-done', { status: 'failed' }),
        run('a', 'f-off', { status: 'cancelled' }),
      ],
      fleet,
    );

    expect(bands['needs-you'].map((listed) => listed.run.featureId)).toEqual(['f-early', 'f-late']);
    expect(bands.active.map((listed) => listed.run.featureId)).toEqual(['f-live']);
    expect(bands.finished.map((listed) => listed.run.featureId)).toEqual(['f-done', 'f-off']);
    expect(bands['needs-you'][0].gate?.stepTitle).toBe('Review diff');
  });

  it('keeps the oldest of two gates on one run', () => {
    const fleet = [instance('a', null, [gate('a', 'f-1', 30), gate('a', 'f-1', 20)])];

    expect(bandRuns([run('a', 'f-1')], fleet)['needs-you'][0].gate?.since).toBe(20);
  });

  it('names each run by its instance label, falling back to the id', () => {
    const bands = bandRuns([run('a', 'f-1'), run('b', 'f-2')], [instance('a', 'workstation', []), instance('b', null, [])]);

    expect(bands.active.map((listed) => listed.instanceLabel).sort()).toEqual(['b', 'workstation']);
  });

  it('bands by status alone when the fleet has not been read', () => {
    const bands = bandRuns([run('a', 'f-1'), run('a', 'f-2', { status: 'completed' })], null);

    expect(bands['needs-you']).toEqual([]);
    expect(bands.active.map((listed) => [listed.run.featureId, listed.instanceLabel])).toEqual([['f-1', 'a']]);
    expect(bands.finished).toHaveLength(1);
  });

  it('newest update first within a band', () => {
    const bands = bandRuns([run('a', 'f-old', { updatedAt: 1 }), run('a', 'f-new', { updatedAt: 9 })], null);

    expect(bands.active.map((listed) => listed.run.featureId)).toEqual(['f-new', 'f-old']);
  });
});

describe('ticketText', () => {
  it('says nothing for a run no ticket has reported on', () => {
    expect(ticketText({ completed: 0, failed: 0, running: 0 })).toBeNull();
  });

  it('counts done tickets against every reported one, and names failures', () => {
    expect(ticketText({ completed: 3, failed: 0, running: 2 })).toBe('3 of 5 tickets done');
    expect(ticketText({ completed: 1, failed: 1, running: 0 })).toBe('1 of 2 tickets done, 1 failed');
  });
});
