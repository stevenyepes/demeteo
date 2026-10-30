import { describe, expect, it } from 'vitest';

import type { FleetInstance, NeedsYou } from './fleet';
import {
  elapsedText,
  fleetSummaryText,
  needsYouOldestFirst,
  platformText,
  redactUserinfo,
  summarizeFleet,
  ticketText,
} from './fleetSummary';

function instance(overrides: Partial<FleetInstance>): FleetInstance {
  return {
    id: 'inst-a',
    label: null,
    present: true,
    os: null,
    arch: null,
    version: null,
    scopes: ['read'],
    kind: 'desktop',
    readGranted: true,
    lastSeen: null,
    liveRuns: 0,
    needsYou: [],
    harnesses: null,
    projectCount: null,
    ...overrides,
  };
}

function gate(featureId: string, since: number): NeedsYou {
  return {
    instanceId: 'unused',
    featureId,
    stepId: null,
    stepExecutionId: `exec-${featureId}`,
    since,
    title: null,
    projectName: null,
    stepTitle: null,
  };
}

describe('summarizeFleet', () => {
  it('counts instances, presence, runs and gates', () => {
    const fleet = [
      instance({ id: 'a', liveRuns: 2, needsYou: [gate('f1', 10)] }),
      instance({ id: 'b', present: false, liveRuns: 1, needsYou: [gate('f2', 20), gate('f3', 30)] }),
    ];

    expect(summarizeFleet(fleet)).toEqual({ instances: 2, online: 1, runsActive: 3, gatesWaiting: 3, withheld: 0 });
  });

  it('reports a withheld instance as withheld, not as zero runs', () => {
    const fleet = [
      instance({ id: 'a', liveRuns: 2, needsYou: [] }),
      instance({ id: 'b', readGranted: false, liveRuns: null, needsYou: null }),
    ];

    expect(summarizeFleet(fleet)).toEqual({ instances: 2, online: 2, runsActive: 2, gatesWaiting: 0, withheld: 1 });
  });

  it('counts an instance withholding only its runs as withheld, and still counts its gates', () => {
    const fleet = [
      instance({ id: 'a', liveRuns: 2, needsYou: [] }),
      instance({ id: 'b', liveRuns: null, needsYou: [gate('f1', 10)] }),
    ];

    expect(summarizeFleet(fleet)).toEqual({ instances: 2, online: 2, runsActive: 2, gatesWaiting: 1, withheld: 1 });
  });

  it('summarizes an empty fleet as all zeros', () => {
    expect(summarizeFleet([])).toEqual({ instances: 0, online: 0, runsActive: 0, gatesWaiting: 0, withheld: 0 });
  });
});

describe('needsYouOldestFirst', () => {
  it('merges two instances oldest first, each with its own label', () => {
    const fleet = [
      instance({ id: 'a', label: 'Laptop', needsYou: [gate('f-new', 300), gate('f-old', 100)] }),
      instance({ id: 'b', label: null, needsYou: [gate('f-mid', 200)] }),
      instance({ id: 'c', needsYou: null }),
    ];

    expect(
      needsYouOldestFirst(fleet).map(({ featureId, instanceId, instanceLabel }) => [
        featureId,
        instanceId,
        instanceLabel,
      ]),
    ).toEqual([
      ['f-old', 'a', 'Laptop'],
      ['f-mid', 'b', 'b'],
      ['f-new', 'a', 'Laptop'],
    ]);
  });
});

describe('elapsedText', () => {
  const now = 1_000_000;

  it.each([
    [now, '0s'],
    [now - 45, '45s'],
    [now - 14 * 60, '14m'],
    [now - 3 * 3600 - 59, '3h'],
    [now - 2 * 86_400, '2d'],
  ])('measures %i against a fixed now as %s', (then, text) => {
    expect(elapsedText(then, now)).toBe(text);
  });

  it('says never for a missing time', () => {
    expect(elapsedText(null, now)).toBe('never');
  });

  it('does not go negative when the instance clock is ahead', () => {
    expect(elapsedText(now + 30, now)).toBe('0s');
  });
});

describe('platformText', () => {
  it('joins os, arch and version', () => {
    expect(platformText({ os: 'linux', arch: 'x86_64', version: '1.4.0' })).toBe('linux · x86_64 · 1.4.0');
  });

  it('skips null parts', () => {
    expect(platformText({ os: 'macos', arch: null, version: '1.4.0' })).toBe('macos · 1.4.0');
    expect(platformText({ os: null, arch: null, version: null })).toBe('');
  });
});

describe('redactUserinfo', () => {
  it('drops user and token from a remote url', () => {
    expect(redactUserinfo('https://user:tok@host/x')).toBe('https://host/x');
  });

  it('leaves a url with no userinfo unchanged', () => {
    expect(redactUserinfo('https://github.com/org/repo.git')).toBe('https://github.com/org/repo.git');
  });

  it('returns a string that is not a url unchanged', () => {
    expect(redactUserinfo('not a url')).toBe('not a url');
  });
});

describe('fleetSummaryText', () => {
  it('pluralises each count on its own', () => {
    expect(fleetSummaryText({ instances: 1, online: 0, runsActive: 1, gatesWaiting: 2, withheld: 0 })).toBe(
      '1 instance · 0 online · 1 run active · 2 need you',
    );
    expect(fleetSummaryText({ instances: 6, online: 4, runsActive: 5, gatesWaiting: 0, withheld: 1 })).toBe(
      '6 instances · 4 online · 5 runs active · 0 need you',
    );
  });
});

describe('ticketText', () => {
  it('says how many landed and failed, and nothing for a run with no tickets', () => {
    expect(ticketText(null)).toBeNull();
    expect(ticketText({ completed: 0, failed: 0, running: 0 })).toBeNull();
    expect(ticketText({ completed: 0, failed: 0, running: 1 })).toBe('0 tickets landed');
    expect(ticketText({ completed: 1, failed: 0, running: 1 })).toBe('1 ticket landed');
    expect(ticketText({ completed: 3, failed: 2, running: 0 })).toBe('3 tickets landed, 2 failed');
  });
});
